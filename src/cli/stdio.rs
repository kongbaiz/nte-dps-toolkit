use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crossbeam_channel::{Receiver, Sender, bounded, select, tick, unbounded};
use serde_json::Value;

use crate::api::PROTOCOL_VERSION;
use crate::api::dto::{
    BattleSummaryDto, BattleSummaryEvent, CaptureDetectResult, InventorySnapshotDto,
    InventorySnapshotEvent,
};
use crate::api::jsonrpc::{
    RpcError, ValidatedRequest, failure, failure_without_id, notification, parse_line, success,
};
use crate::api::request::{
    BattleSummaryParams, CaptureDeviceParam, CaptureProfileParam, CaptureStartParams,
    RawCaptureParam, Request,
};
use crate::api::response::{
    BattleResetResult, CaptureStartResult, CaptureStatusEvent, CaptureStopResult, CoreMessageEvent,
    HelloResult, ShutdownResult, StatusResult,
};
use crate::cli::args::ServeOptions;
use crate::core::capture::{
    self, CaptureController, CaptureControllerOptions, CaptureDeviceSelector, CaptureProfile,
    RawCaptureMode,
};
use crate::core::reducer::{CoreSignal, apply_engine_event};
use crate::core::snapshot::{InventorySnapshot, inventory_snapshot};
use crate::core::{CoreError, CoreErrorCode};
use crate::engine::capture::PacketEmissionMode;
use crate::engine::model::{CaptureQualitySource, CharacterInfo, CombatState, EngineEvent};
use crate::engine::parser::{
    CHARACTER_DATA_PATH, EQUIPMENT_CATALOG_PATH, EquipmentCatalog, load_characters,
    load_equipment_catalog,
};

const MAX_LINE_BYTES: usize = 1024 * 1024;
const COMMAND_QUEUE_CAPACITY: usize = 128;
const OUTBOUND_QUEUE_CAPACITY: usize = 1024;
const BATTLE_SUMMARY_INTERVAL: Duration = Duration::from_millis(250);

enum ReaderEvent {
    Request(ValidatedRequest),
    Error {
        id: Value,
        error: RpcError,
        fatal: bool,
    },
    Eof,
}

enum WriterEvent {
    Closed,
}

enum BoundedLine {
    Eof,
    Line(Vec<u8>),
    TooLong,
}

#[derive(Clone)]
struct LatestMessageSender {
    slot: Arc<Mutex<Option<Value>>>,
    wake: Sender<()>,
}

struct LatestMessageReceiver {
    slot: Arc<Mutex<Option<Value>>>,
    wake: Receiver<()>,
    _wake_guard: Sender<()>,
}

fn latest_message_channel() -> (LatestMessageSender, LatestMessageReceiver) {
    let slot = Arc::new(Mutex::new(None));
    let (wake, wake_receiver) = bounded(1);
    (
        LatestMessageSender {
            slot: Arc::clone(&slot),
            wake: wake.clone(),
        },
        LatestMessageReceiver {
            slot,
            wake: wake_receiver,
            _wake_guard: wake,
        },
    )
}

impl LatestMessageSender {
    fn publish(&self, message: Value) {
        *self.slot.lock().expect("latest message slot lock poisoned") = Some(message);
        let _ = self.wake.try_send(());
    }

    fn clear(&self) {
        *self.slot.lock().expect("latest message slot lock poisoned") = None;
    }
}

impl LatestMessageReceiver {
    fn take(&self) -> Option<Value> {
        self.slot
            .lock()
            .expect("latest message slot lock poisoned")
            .take()
    }
}

struct RuntimeResources {
    characters: Arc<HashMap<u32, CharacterInfo>>,
    equipment_catalog: EquipmentCatalog,
}

impl RuntimeResources {
    fn load() -> anyhow::Result<Self> {
        Ok(Self {
            characters: Arc::new(load_characters(Path::new(CHARACTER_DATA_PATH))?),
            equipment_catalog: load_equipment_catalog(Path::new(EQUIPMENT_CATALOG_PATH))?,
        })
    }
}

struct Runtime {
    handshaken: bool,
    state: CombatState,
    capture: CaptureController,
    characters: Arc<HashMap<u32, CharacterInfo>>,
    equipment_catalog: EquipmentCatalog,
    latest_inventory: Option<InventorySnapshot>,
    sequence: u64,
    inventory_generation: u64,
    operation_sequence: u64,
    active_operation_id: Option<String>,
    running_notified: bool,
    battle_summary_dirty: bool,
    latest_battle_summary: LatestMessageSender,
    engine_sender: Sender<EngineEvent>,
    data_dir: PathBuf,
}

impl Runtime {
    fn new(
        resources: RuntimeResources,
        engine_sender: Sender<EngineEvent>,
        latest_battle_summary: LatestMessageSender,
        data_dir: PathBuf,
    ) -> Self {
        Self {
            handshaken: false,
            state: CombatState::default(),
            capture: CaptureController::default(),
            characters: resources.characters,
            equipment_catalog: resources.equipment_catalog,
            latest_inventory: None,
            sequence: 0,
            inventory_generation: 0,
            operation_sequence: 0,
            active_operation_id: None,
            running_notified: false,
            battle_summary_dirty: false,
            latest_battle_summary,
            engine_sender,
            data_dir,
        }
    }

    fn next_sequence(&mut self) -> u64 {
        self.sequence = self
            .sequence
            .checked_add(1)
            .expect("event sequence cannot overflow during one process lifetime");
        self.sequence
    }

    fn next_operation_id(&mut self) -> String {
        self.operation_sequence = self
            .operation_sequence
            .checked_add(1)
            .expect("operation sequence cannot overflow during one process lifetime");
        format!("capture-{}", self.operation_sequence)
    }

    fn next_inventory_generation(&mut self) -> u64 {
        self.inventory_generation = self
            .inventory_generation
            .checked_add(1)
            .expect("inventory generation cannot overflow during one process lifetime");
        self.inventory_generation
    }

    fn status(&self) -> StatusResult {
        StatusResult::new(
            self.handshaken,
            self.capture.is_running(),
            self.capture.profile().map(CaptureProfile::as_str),
            self.latest_inventory
                .as_ref()
                .map(|snapshot| snapshot.generation),
            !self.state.hits.is_empty()
                || !self.state.stats.is_empty()
                || self.state.abyss.is_active(),
            self.capture
                .raw_capture_path()
                .map(|path| path.display().to_string()),
        )
    }

    fn start_capture(&mut self, params: CaptureStartParams) -> Result<String, CoreError> {
        let profile = match params.profile {
            CaptureProfileParam::Inventory => CaptureProfile::Inventory,
            CaptureProfileParam::Combat => CaptureProfile::Combat,
        };
        let device = match params.device {
            CaptureDeviceParam::Auto => CaptureDeviceSelector::Auto,
            CaptureDeviceParam::Name { name } => CaptureDeviceSelector::Name(name),
        };
        let raw_capture = match params.raw_capture.unwrap_or(RawCaptureParam::Enabled) {
            RawCaptureParam::Enabled => RawCaptureMode::Enabled,
            RawCaptureParam::Disabled => RawCaptureMode::Disabled,
        };
        let expose_raw_capture_path = params.raw_capture == Some(RawCaptureParam::Enabled);
        self.capture.start(
            CaptureControllerOptions {
                profile,
                device,
                include_incoming: params.include_incoming,
                server_damage_calibration: params.server_damage_calibration,
                raw_capture,
                raw_capture_directory: self.data_dir.clone(),
                expose_raw_capture_path,
                packet_emission: PacketEmissionMode::SummaryOnly,
            },
            Arc::clone(&self.characters),
            self.engine_sender.clone(),
        )?;
        let operation_id = self.next_operation_id();
        self.active_operation_id = Some(operation_id.clone());
        self.running_notified = false;
        Ok(operation_id)
    }

    fn stop_capture(&mut self) -> Result<(String, CaptureProfile), CoreError> {
        if !self.capture.is_running() {
            return Err(CoreError::new(
                CoreErrorCode::CaptureNotRunning,
                "capture is not running",
            ));
        }
        let profile = self
            .capture
            .profile()
            .expect("running capture must have a profile");
        let operation_id = self
            .active_operation_id
            .take()
            .expect("running capture must have an operation id");
        self.capture.stop()?;
        self.running_notified = false;
        Ok((operation_id, profile))
    }

    fn process_engine_event(&mut self, event: EngineEvent, outbound: &Sender<Value>) {
        match apply_engine_event(&mut self.state, event) {
            CoreSignal::StateChanged => self.battle_summary_dirty = true,
            CoreSignal::DebugPacket | CoreSignal::PacketObserved => {}
            CoreSignal::InventoryCharactersReplaced => {}
            CoreSignal::InventoryReplaced => {
                let generation = self.next_inventory_generation();
                let snapshot = inventory_snapshot(
                    &self.state.empty_curtain,
                    &self.equipment_catalog,
                    &self.characters,
                    generation,
                    unix_time_ms(),
                );
                self.latest_inventory = Some(snapshot.clone());
                let sequence = self.next_sequence();
                let _ = outbound.send(notification(
                    "event.inventory.snapshot",
                    InventorySnapshotEvent {
                        sequence,
                        snapshot: InventorySnapshotDto::from(&snapshot),
                    },
                ));
            }
            CoreSignal::Status(_) => {
                if self.capture.is_running() && !self.running_notified {
                    self.running_notified = true;
                    self.send_capture_status(outbound, "running");
                }
            }
            CoreSignal::Warning(_) => {
                let sequence = self.next_sequence();
                let _ = outbound.send(notification(
                    "event.core.warning",
                    CoreMessageEvent {
                        sequence,
                        message: "Capture warning",
                    },
                ));
            }
            CoreSignal::Error(_) => {
                let sequence = self.next_sequence();
                let _ = outbound.send(notification(
                    "event.core.error",
                    CoreMessageEvent {
                        sequence,
                        message: "Capture failed",
                    },
                ));
            }
            CoreSignal::CaptureStopped => {
                if self.capture.is_running() {
                    let profile = self
                        .capture
                        .profile()
                        .expect("running capture must have a profile");
                    let operation_id = self
                        .active_operation_id
                        .take()
                        .expect("running capture must have an operation id");
                    self.capture.capture_stopped();
                    self.running_notified = false;
                    self.send_final_battle_summary(outbound);
                    self.send_capture_status_for(outbound, operation_id, profile, "stopped");
                }
            }
        }
    }

    fn battle_summary(&self, subtract_time_stop: bool) -> Option<BattleSummaryDto> {
        let dps_time_mode = if subtract_time_stop {
            "subtract_time_stop"
        } else {
            "wall_clock"
        };
        self.state
            .session_summary(
                CaptureQualitySource::Live,
                dps_time_mode,
                subtract_time_stop,
            )
            .as_ref()
            .map(BattleSummaryDto::from)
    }

    fn flush_battle_summary(&mut self) {
        if !self.battle_summary_dirty {
            return;
        }
        self.battle_summary_dirty = false;
        let Some(summary) = self.battle_summary(true) else {
            return;
        };
        let sequence = self.next_sequence();
        self.latest_battle_summary.publish(notification(
            "event.battle.summary",
            BattleSummaryEvent { sequence, summary },
        ));
    }

    fn send_final_battle_summary(&mut self, outbound: &Sender<Value>) {
        self.battle_summary_dirty = false;
        self.latest_battle_summary.clear();
        let Some(summary) = self.battle_summary(true) else {
            return;
        };
        let sequence = self.next_sequence();
        let _ = outbound.send(notification(
            "event.battle.summary",
            BattleSummaryEvent { sequence, summary },
        ));
    }

    fn reset_battle(&mut self) {
        self.state.clear_battle_preserving_inventory();
        self.battle_summary_dirty = false;
        self.latest_battle_summary.clear();
    }

    fn send_capture_status(&mut self, outbound: &Sender<Value>, status: &'static str) {
        let operation_id = self
            .active_operation_id
            .clone()
            .expect("running capture must have an operation id");
        let profile = self
            .capture
            .profile()
            .expect("running capture must have a profile");
        self.send_capture_status_for(outbound, operation_id, profile, status);
    }

    fn send_capture_status_for(
        &mut self,
        outbound: &Sender<Value>,
        operation_id: String,
        profile: CaptureProfile,
        status: &'static str,
    ) {
        let sequence = self.next_sequence();
        let _ = outbound.send(notification(
            "event.capture.status",
            CaptureStatusEvent {
                sequence,
                operation_id,
                status,
                profile: profile.as_str(),
            },
        ));
    }
}

pub fn serve(options: ServeOptions) -> i32 {
    run(io::stdin(), io::stdout(), options.data_dir)
}

fn run<R, W>(reader: R, writer: W, data_dir: PathBuf) -> i32
where
    R: Read + Send + 'static,
    W: Write + Send + 'static,
{
    let resources = match RuntimeResources::load() {
        Ok(resources) => resources,
        Err(_) => {
            eprintln!("error: failed to load core data resources");
            return 1;
        }
    };
    let (command_tx, command_rx) = bounded(COMMAND_QUEUE_CAPACITY);
    let (outbound_tx, outbound_rx) = bounded(OUTBOUND_QUEUE_CAPACITY);
    let (writer_event_tx, writer_event_rx) = unbounded();
    let (engine_sender, engine_receiver) = unbounded();
    let (latest_battle_sender, latest_battle_receiver) = latest_message_channel();
    let runtime = Runtime::new(resources, engine_sender, latest_battle_sender, data_dir);

    thread::spawn(move || reader_loop(reader, command_tx));
    let writer_thread = thread::spawn(move || {
        writer_loop(writer, outbound_rx, latest_battle_receiver, writer_event_tx)
    });

    core_loop(
        command_rx,
        &outbound_tx,
        writer_event_rx,
        engine_receiver,
        runtime,
    );
    drop(outbound_tx);
    let _ = writer_thread.join();
    0
}

fn reader_loop<R: Read>(reader: R, sender: Sender<ReaderEvent>) {
    let mut reader = BufReader::new(reader);
    loop {
        match read_bounded_line(&mut reader) {
            Ok(BoundedLine::Eof) => {
                let _ = sender.send(ReaderEvent::Eof);
                return;
            }
            Ok(BoundedLine::TooLong) => {
                let _ = sender.send(ReaderEvent::Error {
                    id: Value::Null,
                    error: RpcError::invalid_request(),
                    fatal: true,
                });
                return;
            }
            Ok(BoundedLine::Line(line)) => {
                let Ok(line) = std::str::from_utf8(&line) else {
                    if sender
                        .send(ReaderEvent::Error {
                            id: Value::Null,
                            error: RpcError::parse_error(),
                            fatal: false,
                        })
                        .is_err()
                    {
                        return;
                    }
                    continue;
                };
                if line.trim().is_empty() {
                    continue;
                }
                let event = match parse_line(line) {
                    Ok(request) => ReaderEvent::Request(request),
                    Err(failure) => ReaderEvent::Error {
                        id: failure.id,
                        error: failure.error,
                        fatal: false,
                    },
                };
                if sender.send(event).is_err() {
                    return;
                }
            }
            Err(_) => {
                let _ = sender.send(ReaderEvent::Eof);
                return;
            }
        }
    }
}

fn read_bounded_line<R: BufRead>(reader: &mut R) -> io::Result<BoundedLine> {
    let mut line = Vec::new();
    loop {
        let buffer = reader.fill_buf()?;
        if buffer.is_empty() {
            return if line.is_empty() {
                Ok(BoundedLine::Eof)
            } else {
                Ok(BoundedLine::Line(line))
            };
        }
        let newline = buffer.iter().position(|byte| *byte == b'\n');
        let take = newline.map_or(buffer.len(), |index| index + 1);
        line.extend_from_slice(&buffer[..take]);
        reader.consume(take);

        if newline.is_some() {
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            return if line.len() > MAX_LINE_BYTES {
                Ok(BoundedLine::TooLong)
            } else {
                Ok(BoundedLine::Line(line))
            };
        }
        if line.len() > MAX_LINE_BYTES {
            return Ok(BoundedLine::TooLong);
        }
    }
}

fn writer_loop<W: Write>(
    mut writer: W,
    receiver: Receiver<Value>,
    latest_battle: LatestMessageReceiver,
    event_sender: Sender<WriterEvent>,
) {
    loop {
        select! {
            recv(receiver) -> message => match message {
                Ok(message) => {
                    if write_message(&mut writer, &message).is_err() {
                        let _ = event_sender.send(WriterEvent::Closed);
                        return;
                    }
                }
                Err(_) => {
                    if let Some(message) = latest_battle.take() {
                        let _ = write_message(&mut writer, &message);
                    }
                    return;
                }
            },
            recv(latest_battle.wake) -> _ => {
                if let Some(message) = latest_battle.take()
                    && write_message(&mut writer, &message).is_err()
                {
                    let _ = event_sender.send(WriterEvent::Closed);
                    return;
                }
            }
        }
    }
}

fn write_message(writer: &mut impl Write, message: &Value) -> io::Result<()> {
    serde_json::to_writer(&mut *writer, message).map_err(io::Error::other)?;
    writer.write_all(b"\n")?;
    writer.flush()
}

fn core_loop(
    command_rx: Receiver<ReaderEvent>,
    outbound_tx: &Sender<Value>,
    writer_event_rx: Receiver<WriterEvent>,
    engine_receiver: Receiver<EngineEvent>,
    mut runtime: Runtime,
) {
    let battle_summary_tick = tick(BATTLE_SUMMARY_INTERVAL);
    loop {
        select! {
            recv(writer_event_rx) -> _ => {
                runtime.capture.stop_if_running();
                return;
            },
            recv(engine_receiver) -> event => {
                if let Ok(event) = event {
                    runtime.process_engine_event(event, outbound_tx);
                }
            },
            recv(battle_summary_tick) -> _ => runtime.flush_battle_summary(),
            recv(command_rx) -> event => {
                let Ok(event) = event else {
                    stop_for_exit(&mut runtime, &engine_receiver, outbound_tx);
                    return;
                };
                match event {
                    ReaderEvent::Eof => {
                        stop_for_exit(&mut runtime, &engine_receiver, outbound_tx);
                        return;
                    }
                    ReaderEvent::Error { id, error, fatal } => {
                        let message = if id.is_null() {
                            failure_without_id(error)
                        } else {
                            failure(id, error)
                        };
                        if outbound_tx.send(message).is_err() || fatal {
                            stop_for_exit(&mut runtime, &engine_receiver, outbound_tx);
                            return;
                        }
                    }
                    ReaderEvent::Request(request) => {
                        if handle_request(
                            request,
                            &mut runtime,
                            &engine_receiver,
                            outbound_tx,
                        ) {
                            return;
                        }
                    }
                }
            }
        }
    }
}

fn handle_request(
    request: ValidatedRequest,
    runtime: &mut Runtime,
    engine_receiver: &Receiver<EngineEvent>,
    outbound: &Sender<Value>,
) -> bool {
    let id = request.id;
    match request.request {
        Request::Hello(params) => {
            if params.protocol_min > PROTOCOL_VERSION || params.protocol_max < PROTOCOL_VERSION {
                return send(
                    outbound,
                    failure(
                        id,
                        RpcError::domain(
                            "PROTOCOL_VERSION_MISMATCH",
                            format!("Supported protocol version is {PROTOCOL_VERSION}"),
                        ),
                    ),
                );
            }
            runtime.handshaken = true;
            send(outbound, success(id, HelloResult::default()))
        }
        Request::Shutdown => {
            stop_for_exit(runtime, engine_receiver, outbound);
            let _ = outbound.send(success(
                id,
                ShutdownResult {
                    shutting_down: true,
                },
            ));
            true
        }
        _ if !runtime.handshaken => send(
            outbound,
            failure(
                id,
                RpcError::domain(
                    "HANDSHAKE_REQUIRED",
                    "core.hello must succeed before this method",
                ),
            ),
        ),
        Request::Status => send(outbound, success(id, runtime.status())),
        Request::CaptureDetect => {
            let message = match capture::detect_environment() {
                Ok(environment) => success(id, CaptureDetectResult::from(environment)),
                Err(error) => failure(id, core_error(error.code)),
            };
            send(outbound, message)
        }
        Request::CaptureStart(params) => match runtime.start_capture(params) {
            Ok(operation_id) => {
                let response = success(
                    id,
                    CaptureStartResult {
                        operation_id: operation_id.clone(),
                    },
                );
                if outbound.send(response).is_err() {
                    return true;
                }
                runtime.send_capture_status(outbound, "starting");
                false
            }
            Err(error) => send(outbound, failure(id, core_error(error.code))),
        },
        Request::CaptureStop => {
            let result = runtime.stop_capture();
            drain_engine_events(runtime, engine_receiver, outbound);
            match result {
                Ok((operation_id, profile)) => {
                    runtime.send_final_battle_summary(outbound);
                    let response = success(
                        id,
                        CaptureStopResult {
                            operation_id: operation_id.clone(),
                            stopped: true,
                        },
                    );
                    if outbound.send(response).is_err() {
                        return true;
                    }
                    runtime.send_capture_status_for(outbound, operation_id, profile, "stopped");
                    false
                }
                Err(error) => send(outbound, failure(id, core_error(error.code))),
            }
        }
        Request::InventoryGetLatest => {
            let message = match runtime.latest_inventory.as_ref() {
                Some(snapshot) => success(id, InventorySnapshotDto::from(snapshot)),
                None => failure(
                    id,
                    RpcError::domain(
                        "INVENTORY_NOT_READY",
                        "No complete inventory snapshot is available",
                    ),
                ),
            };
            send(outbound, message)
        }
        Request::BattleGetSummary(BattleSummaryParams { subtract_time_stop }) => {
            drain_engine_events(runtime, engine_receiver, outbound);
            send(
                outbound,
                success(id, runtime.battle_summary(subtract_time_stop)),
            )
        }
        Request::BattleReset => {
            drain_engine_events(runtime, engine_receiver, outbound);
            runtime.reset_battle();
            send(outbound, success(id, BattleResetResult { reset: true }))
        }
        Request::Unknown => send(outbound, failure(id, RpcError::method_not_found())),
    }
}

fn stop_for_exit(
    runtime: &mut Runtime,
    engine_receiver: &Receiver<EngineEvent>,
    outbound: &Sender<Value>,
) {
    if runtime.capture.is_running() {
        let (operation_id, profile) = runtime
            .stop_capture()
            .expect("capture checked as running must stop");
        drain_engine_events(runtime, engine_receiver, outbound);
        runtime.send_final_battle_summary(outbound);
        runtime.send_capture_status_for(outbound, operation_id, profile, "stopped");
    }
}

fn drain_engine_events(
    runtime: &mut Runtime,
    engine_receiver: &Receiver<EngineEvent>,
    outbound: &Sender<Value>,
) {
    while let Ok(event) = engine_receiver.try_recv() {
        runtime.process_engine_event(event, outbound);
    }
}

fn send(outbound: &Sender<Value>, message: Value) -> bool {
    outbound.send(message).is_err()
}

fn core_error(code: CoreErrorCode) -> RpcError {
    match code {
        CoreErrorCode::NpcapNotFound => RpcError::domain(
            "NPCAP_NOT_FOUND",
            "Npcap is unavailable or device enumeration failed",
        ),
        CoreErrorCode::GameProcessNotFound => RpcError::domain(
            "GAME_PROCESS_NOT_FOUND",
            "The game process was not detected",
        ),
        CoreErrorCode::CaptureDeviceNotFound => RpcError::domain(
            "CAPTURE_DEVICE_NOT_FOUND",
            "The requested capture device was not found",
        ),
        CoreErrorCode::SystemProbeFailed => {
            RpcError::domain("SYSTEM_PROBE_FAILED", "The system environment probe failed")
        }
        CoreErrorCode::CaptureAlreadyRunning => {
            RpcError::domain("CAPTURE_ALREADY_RUNNING", "A capture is already running")
        }
        CoreErrorCode::CaptureNotRunning => {
            RpcError::domain("CAPTURE_NOT_RUNNING", "No capture is running")
        }
    }
}

fn unix_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    use crate::engine::model::{EmptyCurtainItem, Hit, HtItemNetId, PacketDebug, TimeStopEvent};

    #[test]
    fn bounded_reader_accepts_limit_and_rejects_larger_lines() {
        let mut exact = Cursor::new(vec![b'a'; MAX_LINE_BYTES]);
        assert!(matches!(
            read_bounded_line(&mut exact).unwrap(),
            BoundedLine::Line(line) if line.len() == MAX_LINE_BYTES
        ));

        let mut larger = Cursor::new(vec![b'a'; MAX_LINE_BYTES + 1]);
        assert!(matches!(
            read_bounded_line(&mut larger).unwrap(),
            BoundedLine::TooLong
        ));
    }

    #[test]
    fn invalid_json_does_not_stop_following_requests() {
        let input = b"not-json\n{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"core.shutdown\"}\n";
        let output = SharedWriter::default();
        let captured = output.clone();
        assert_eq!(run(Cursor::new(input), output, PathBuf::from("logs")), 0);
        let lines: Vec<Value> = String::from_utf8(captured.bytes())
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["error"]["code"], -32700);
        assert_eq!(lines[1]["id"], 1);
    }

    #[test]
    fn inventory_event_is_enriched_and_packet_debug_is_not_forwarded() {
        let resources = RuntimeResources::load().unwrap();
        let (engine_sender, engine_receiver) = unbounded();
        let (latest_battle, _) = latest_message_channel();
        let mut runtime = Runtime::new(
            resources,
            engine_sender.clone(),
            latest_battle,
            PathBuf::from("logs"),
        );
        let (outbound, receiver) = bounded(4);
        engine_sender
            .send(EngineEvent::EmptyCurtain(vec![EmptyCurtainItem {
                id: HtItemNetId { solt: 4, serial: 5 },
                item_id: "Attack_blue".to_owned(),
                level: 20,
                main_stats: Vec::new(),
                sub_stats: Vec::new(),
                locked: true,
                character_net_id: Some(HtItemNetId { solt: 6, serial: 7 }),
                equipped_character_id: Some(1020),
            }]))
            .unwrap();
        drain_engine_events(&mut runtime, &engine_receiver, &outbound);
        let inventory = receiver.recv().unwrap();
        assert_eq!(inventory["method"], "event.inventory.snapshot");
        assert_eq!(inventory["params"]["generation"], 1);
        assert_eq!(inventory["params"]["item_count"], 1);
        assert_eq!(inventory["params"]["items"][0]["uid"]["slot"], 4);
        assert_eq!(
            inventory["params"]["items"][0]["equipped_character_id"],
            1020
        );
        assert_eq!(
            inventory["params"]["items"][0]["equipped_character_uid"]["slot"],
            6
        );
        assert_eq!(
            inventory["params"]["items"][0]["names"]["en"],
            "Shadow Creed"
        );
        assert_eq!(runtime.latest_inventory.as_ref().unwrap().generation, 1);

        runtime.process_engine_event(
            EngineEvent::Packet(Box::new(PacketDebug {
                timestamp: 1.0,
                source: "source".to_owned(),
                destination: "destination".to_owned(),
                direction: "outgoing".to_owned(),
                payload_len: 1,
                declared_ids: Vec::new(),
                parsed_hits: 0,
                note: String::new(),
                payload_preview: "private".to_owned(),
                payload_hex: "00".to_owned(),
                decoded_text: "private".to_owned(),
            })),
            &outbound,
        );
        assert!(receiver.try_recv().is_err());

        runtime.process_engine_event(EngineEvent::Warning("private detail".to_owned()), &outbound);
        let warning = receiver.recv().unwrap();
        assert_eq!(warning["method"], "event.core.warning");
        assert_eq!(warning["params"]["sequence"], 2);
        assert_eq!(warning["params"]["message"], "Capture warning");
        assert!(!warning.to_string().contains("private detail"));

        runtime.process_engine_event(EngineEvent::Error("private failure".to_owned()), &outbound);
        let error = receiver.recv().unwrap();
        assert_eq!(error["method"], "event.core.error");
        assert_eq!(error["params"]["sequence"], 3);
        assert_eq!(error["params"]["message"], "Capture failed");
        assert!(!error.to_string().contains("private failure"));

        runtime.send_capture_status_for(
            &outbound,
            "capture-test".to_owned(),
            CaptureProfile::Inventory,
            "stopped",
        );
        let status = receiver.recv().unwrap();
        assert_eq!(status["method"], "event.capture.status");
        assert_eq!(status["params"]["sequence"], 4);
        assert_eq!(status["params"]["operation_id"], "capture-test");
        assert_eq!(status["params"]["status"], "stopped");
    }

    #[test]
    fn battle_summary_tick_coalesces_latest_and_final_is_reliable() {
        assert_eq!(BATTLE_SUMMARY_INTERVAL, Duration::from_millis(250));
        let resources = RuntimeResources::load().unwrap();
        let (engine_sender, _) = unbounded();
        let (latest_battle, latest_receiver) = latest_message_channel();
        let mut runtime = Runtime::new(
            resources,
            engine_sender,
            latest_battle,
            PathBuf::from("logs"),
        );
        let (outbound, reliable) = bounded(4);

        runtime.process_engine_event(EngineEvent::Hit(Box::new(test_hit(1.0, 100.0))), &outbound);
        runtime.flush_battle_summary();
        runtime.process_engine_event(
            EngineEvent::TimeStop(TimeStopEvent::UltraAnimation {
                timestamp: 2.0,
                char_id: 7,
                ability_id: "test-ultra".to_owned(),
                duration_seconds: 2.0,
            }),
            &outbound,
        );
        runtime.process_engine_event(EngineEvent::Hit(Box::new(test_hit(10.0, 200.0))), &outbound);
        runtime.flush_battle_summary();

        let latest = latest_receiver.take().unwrap();
        assert_eq!(latest["method"], "event.battle.summary");
        assert_eq!(latest["params"]["sequence"], 2);
        assert_eq!(latest["params"]["total_damage"], 300.0);
        assert_eq!(latest["params"]["dps_time_mode"], "subtract_time_stop");
        assert_eq!(latest["params"]["duration_seconds"], 7.0);
        assert!(latest_receiver.take().is_none());
        assert!(reliable.try_recv().is_err());
        let wall_clock = runtime.battle_summary(false).unwrap();
        assert_eq!(wall_clock.dps_time_mode, "wall_clock");
        assert_eq!(wall_clock.duration_seconds, 9.0);

        runtime.send_final_battle_summary(&outbound);
        let final_summary = reliable.recv().unwrap();
        assert_eq!(final_summary["method"], "event.battle.summary");
        assert_eq!(final_summary["params"]["sequence"], 3);
        assert_eq!(final_summary["params"]["total_damage"], 300.0);
    }

    #[test]
    fn battle_reset_keeps_latest_inventory_and_clears_pending_summary() {
        let resources = RuntimeResources::load().unwrap();
        let (engine_sender, _) = unbounded();
        let (latest_battle, latest_receiver) = latest_message_channel();
        let mut runtime = Runtime::new(
            resources,
            engine_sender,
            latest_battle,
            PathBuf::from("logs"),
        );
        let (outbound, receiver) = bounded(4);
        runtime.process_engine_event(
            EngineEvent::EmptyCurtain(vec![EmptyCurtainItem {
                id: HtItemNetId { solt: 1, serial: 2 },
                item_id: "Attack_blue".to_owned(),
                level: 20,
                main_stats: Vec::new(),
                sub_stats: Vec::new(),
                locked: false,
                character_net_id: None,
                equipped_character_id: None,
            }]),
            &outbound,
        );
        receiver.recv().unwrap();
        runtime.process_engine_event(EngineEvent::Hit(Box::new(test_hit(1.0, 100.0))), &outbound);
        runtime.flush_battle_summary();

        runtime.reset_battle();

        assert!(runtime.state.hits.is_empty());
        assert!(runtime.latest_inventory.is_some());
        assert_eq!(runtime.state.empty_curtain.len(), 1);
        assert!(latest_receiver.take().is_none());
        assert!(runtime.battle_summary(true).is_none());
    }

    #[test]
    fn battle_rpc_drains_queued_events_before_query_and_reset() {
        let resources = RuntimeResources::load().unwrap();
        let (engine_sender, engine_receiver) = unbounded();
        let (latest_battle, _) = latest_message_channel();
        let mut runtime = Runtime::new(
            resources,
            engine_sender.clone(),
            latest_battle,
            PathBuf::from("logs"),
        );
        runtime.handshaken = true;
        let (outbound, receiver) = bounded(4);
        engine_sender
            .send(EngineEvent::Hit(Box::new(test_hit(1.0, 100.0))))
            .unwrap();

        assert!(!handle_request(
            ValidatedRequest {
                id: serde_json::json!(1),
                request: Request::BattleGetSummary(BattleSummaryParams {
                    subtract_time_stop: true,
                }),
            },
            &mut runtime,
            &engine_receiver,
            &outbound,
        ));
        let summary = receiver.recv().unwrap();
        assert_eq!(summary["result"]["total_damage"], 100.0);

        engine_sender
            .send(EngineEvent::Hit(Box::new(test_hit(2.0, 200.0))))
            .unwrap();
        assert!(!handle_request(
            ValidatedRequest {
                id: serde_json::json!(2),
                request: Request::BattleReset,
            },
            &mut runtime,
            &engine_receiver,
            &outbound,
        ));
        let reset = receiver.recv().unwrap();
        assert_eq!(reset["result"]["reset"], true);
        assert!(runtime.state.hits.is_empty());
        assert!(runtime.battle_summary(true).is_none());
    }

    #[test]
    fn stdout_writer_keeps_only_the_latest_coalesced_summary() {
        let (reliable_sender, reliable_receiver) = bounded(1);
        let (latest_sender, latest_receiver) = latest_message_channel();
        latest_sender.publish(serde_json::json!({"generation": 1}));
        latest_sender.publish(serde_json::json!({"generation": 2}));
        drop(reliable_sender);
        let output = SharedWriter::default();
        let captured = output.clone();
        let (writer_event, _) = unbounded();

        writer_loop(output, reliable_receiver, latest_receiver, writer_event);

        let lines: Vec<Value> = String::from_utf8(captured.bytes())
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(lines, vec![serde_json::json!({"generation": 2})]);
    }

    fn test_hit(timestamp: f64, damage: f64) -> Hit {
        Hit {
            timestamp,
            char_id: 7,
            char_name: "Character".to_owned(),
            char_known: true,
            damage,
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
            damage_name: Some("Skill".to_owned()),
            attack_type: Some("normal".to_owned()),
            damage_attribute: None,
            follow_up_damage: 0.0,
            follow_up_timestamp: None,
            follow_up_damage_name: None,
            follow_up_attack_type: None,
            follow_up_damage_attribute: None,
        }
    }

    #[derive(Clone, Default)]
    struct SharedWriter(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl SharedWriter {
        fn bytes(&self) -> Vec<u8> {
            self.0.lock().unwrap().clone()
        }
    }

    impl Write for SharedWriter {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
}
