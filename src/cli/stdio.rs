use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crossbeam_channel::{Receiver, Sender, bounded, never, select, tick, unbounded};
use serde_json::Value;

use crate::api::PROTOCOL_VERSION;
use crate::api::battle::{
    BattleAxisDto, BattleReadError, BattleRecordContext, BattleRecordDto, BattleTimelineDto,
    battle_axis, battle_record, battle_timeline,
};
use crate::api::dto::{
    BattleSummaryDto, BattleSummaryEvent, CaptureDetectResult, InventorySnapshotDto,
    InventorySnapshotEvent,
};
use crate::api::jsonrpc::{
    RpcError, ValidatedRequest, failure, failure_without_id, notification, parse_line, success,
};
use crate::api::request::{
    BattleAxisParams, BattleRecordParams, BattleSummaryParams, BattleTimelineParams,
    BattleTimelineScopeParam, CaptureDeviceParam, CaptureProfileParam, CaptureStartParams,
    EquipmentOperationParam, ItemUidParam, RawCaptureParam, Request,
};
use crate::api::response::{
    BattleResetResult, CaptureStartResult, CaptureStatusEvent, CaptureStopResult, CoreMessageEvent,
    EquipmentRequestResult, HelloResult, ShutdownResult, StatusResult,
};
use crate::cli::args::ServeOptions;
use crate::core::capture::{
    self, CaptureController, CaptureControllerOptions, CaptureDeviceSelector, CaptureProfile,
    RawCaptureMode,
};
use crate::core::reducer::{CoreSignal, apply_engine_event};
use crate::core::snapshot::{InventorySnapshot, inventory_snapshot};
use crate::core::timeline::TimelineScope;
use crate::core::{CoreError, CoreErrorCode};
use crate::engine::capture::PacketEmissionMode;
use crate::engine::model::{
    CaptureQualitySource, CharacterInfo, CombatState, DpsTimeBasis, EngineEvent, HtItemNetId,
    TimeStopEvent,
};
use crate::engine::parser::{
    AbilityCatalog, CHARACTER_DATA_PATH, EQUIPMENT_CATALOG_PATH, EquipmentCatalog,
    GAMEPLAY_EFFECT_SEMANTICS_PATH, SKILL_DAMAGE_DATA_PATH, load_characters,
    load_equipment_catalog,
};
use crate::platform::mods_plugin::{
    ModsPluginClient, ModsPluginOperation, ModsPluginPlacement, ModsPluginRequest,
    ModsPluginResponse, ModsPluginSubmitError,
};

const MAX_LINE_BYTES: usize = 1024 * 1024;
const COMMAND_QUEUE_CAPACITY: usize = 128;
const OUTBOUND_QUEUE_CAPACITY: usize = 1024;
/// CLI EngineEvent queue policy:
/// - capacity: 16,384 events;
/// - ordering: FIFO for the single capture/replay producer lane;
/// - full: reliable semantic events backpressure the producer rather than drop;
/// - disconnect: producer send fails and its capture task terminates;
/// - debug: CLI capture uses SummaryOnly, so no large FullDebug event lane is
///   accumulated while stdout is backpressured.
const ENGINE_EVENT_QUEUE_CAPACITY: usize = 16_384;
const BATTLE_SUMMARY_INTERVAL: Duration = Duration::from_millis(250);
const LIVE_BATTLE_READ_EVENT_DRAIN_LIMIT: usize = 512;

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

/// The coalescing slot is ephemeral. A panic can leave an arbitrary partial
/// JSON value in it, so reset to empty and clear poison before reuse.
fn lock_latest_message_slot(
    slot: &Mutex<Option<Value>>,
) -> std::sync::MutexGuard<'_, Option<Value>> {
    match slot.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            let mut guard = poisoned.into_inner();
            *guard = None;
            slot.clear_poison();
            guard
        }
    }
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
        *lock_latest_message_slot(&self.slot) = Some(message);
        let _ = self.wake.try_send(());
    }

    fn clear(&self) {
        *lock_latest_message_slot(&self.slot) = None;
    }
}

impl LatestMessageReceiver {
    fn take(&self) -> Option<Value> {
        lock_latest_message_slot(&self.slot).take()
    }
}

struct RuntimeResources {
    characters: Arc<HashMap<u32, CharacterInfo>>,
    ability_catalog: Arc<AbilityCatalog>,
    equipment_catalog: EquipmentCatalog,
}

impl RuntimeResources {
    fn load() -> anyhow::Result<Self> {
        let mut ability_catalog = AbilityCatalog::load(Path::new(SKILL_DAMAGE_DATA_PATH))?;
        ability_catalog.apply_semantics(Path::new(GAMEPLAY_EFFECT_SEMANTICS_PATH))?;
        Ok(Self {
            characters: Arc::new(load_characters(Path::new(CHARACTER_DATA_PATH))?),
            ability_catalog: Arc::new(ability_catalog),
            equipment_catalog: load_equipment_catalog(Path::new(EQUIPMENT_CATALOG_PATH))?,
        })
    }
}

struct Runtime {
    handshaken: bool,
    state: CombatState,
    capture: CaptureController,
    characters: Arc<HashMap<u32, CharacterInfo>>,
    ability_catalog: Arc<AbilityCatalog>,
    equipment_catalog: EquipmentCatalog,
    latest_inventory: Option<InventorySnapshot>,
    sequence: u64,
    inventory_generation: u64,
    operation_sequence: u64,
    battle_record_sequence: u64,
    battle_record: Option<BattleRecordRuntime>,
    equipment_request_sequence: u64,
    mods_plugin: ModsPluginClient,
    pending_equipment_requests: HashMap<u64, Value>,
    active_operation_id: Option<String>,
    latest_operation_id: Option<String>,
    running_notified: bool,
    battle_summary_dirty: bool,
    latest_battle_summary: LatestMessageSender,
    engine_sender: Sender<EngineEvent>,
    data_dir: PathBuf,
}

#[derive(Clone, Debug)]
struct BattleRecordRuntime {
    id: String,
    capture_operation_id: Option<String>,
    generation: u64,
    finalized: bool,
    finalized_at_unix_ms: Option<u64>,
    axis_base_sequence: u64,
    source: CaptureQualitySource,
}

impl BattleRecordRuntime {
    fn context(&self) -> BattleRecordContext<'_> {
        BattleRecordContext {
            id: &self.id,
            capture_operation_id: self.capture_operation_id.as_deref(),
            generation: self.generation,
            finalized: self.finalized,
            finalized_at_unix_ms: self.finalized_at_unix_ms,
            axis_base_sequence: self.axis_base_sequence,
            source: self.source,
        }
    }
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
            ability_catalog: resources.ability_catalog,
            equipment_catalog: resources.equipment_catalog,
            latest_inventory: None,
            sequence: 0,
            inventory_generation: 0,
            operation_sequence: 0,
            battle_record_sequence: 0,
            battle_record: None,
            equipment_request_sequence: 0,
            mods_plugin: ModsPluginClient::new(),
            pending_equipment_requests: HashMap::new(),
            active_operation_id: None,
            latest_operation_id: None,
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

    fn publish_inventory_snapshot(&mut self, outbound: &Sender<Value>) {
        let generation = self.next_inventory_generation();
        let snapshot = inventory_snapshot(
            &self.state.empty_curtain,
            &self.state.empty_curtain_characters,
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

    fn next_equipment_request_id(&mut self) -> u64 {
        self.equipment_request_sequence = self
            .equipment_request_sequence
            .checked_add(1)
            .expect("equipment request sequence cannot overflow during one process lifetime");
        self.equipment_request_sequence
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

    fn submit_equipment_request(
        &mut self,
        id: Value,
        request: ModsPluginRequest,
    ) -> Result<(), ModsPluginSubmitError> {
        let request_id = request.request_id;
        self.mods_plugin.submit_request(request)?;
        assert!(
            self.pending_equipment_requests
                .insert(request_id, id)
                .is_none(),
            "equipment request IDs must be unique during one process lifetime"
        );
        Ok(())
    }

    fn process_equipment_response(
        &mut self,
        response: ModsPluginResponse,
        outbound: &Sender<Value>,
    ) -> bool {
        let Some(id) = self.pending_equipment_requests.remove(&response.request_id) else {
            eprintln!(
                "warning: ignoring Mod loader response for unknown or stale request_id {}",
                response.request_id
            );
            return false;
        };
        let message = match response.status {
            Ok(0) => success(
                id,
                EquipmentRequestResult {
                    status: "rpc_dispatched",
                },
            ),
            Ok(1) => success(
                id,
                EquipmentRequestResult {
                    status: "dry_run_ok",
                },
            ),
            Ok(status) => failure(
                id,
                RpcError::domain(
                    "EQUIPMENT_REQUEST_REJECTED",
                    format!("Mod loader rejected the request with status {status}"),
                ),
            ),
            Err(_) => failure(
                id,
                RpcError::domain("MODS_PLUGIN_UNAVAILABLE", "Mod loader is unavailable"),
            ),
        };
        send(outbound, message)
    }

    fn fail_pending_equipment_requests(&mut self, outbound: &Sender<Value>) -> bool {
        let pending = std::mem::take(&mut self.pending_equipment_requests);
        if pending.is_empty() {
            eprintln!(
                "warning: Mod loader worker disconnected; no equipment requests were pending"
            );
            return false;
        }
        eprintln!(
            "warning: Mod loader worker disconnected; failing {} pending equipment request(s)",
            pending.len()
        );
        for (_, id) in pending {
            if outbound
                .send(failure(
                    id,
                    RpcError::domain("MODS_PLUGIN_UNAVAILABLE", "Mod loader is unavailable"),
                ))
                .is_err()
            {
                return true;
            }
        }
        false
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
                filter: "udp".to_owned(),
                include_incoming: params.include_incoming,
                server_damage_calibration: params.server_damage_calibration,
                raw_capture,
                raw_capture_directory: self.data_dir.clone(),
                expose_raw_capture_path,
                packet_emission: PacketEmissionMode::SummaryOnly,
            },
            Arc::clone(&self.characters),
            Arc::clone(&self.ability_catalog),
            self.engine_sender.clone(),
        )?;
        let operation_id = self.next_operation_id();
        self.active_operation_id = Some(operation_id.clone());
        self.latest_operation_id = Some(operation_id.clone());
        self.running_notified = false;
        Ok(operation_id)
    }

    fn stop_capture_with_drain(
        &mut self,
        engine_receiver: &Receiver<EngineEvent>,
    ) -> Result<(String, CaptureProfile), CoreError> {
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
        // Move the controller out so the drain callback can keep reducing
        // authoritative events into the rest of `self` while join waits. A
        // disconnected sink preserves reducer semantics but makes every
        // presentation notification an immediate no-op: stdout backpressure
        // must never prevent the reliable engine lane from being drained.
        let mut controller = std::mem::take(&mut self.capture);
        let (discard_outbound, discard_receiver) = bounded(0);
        drop(discard_receiver);
        let stop_result = controller.stop_with_drain(|| {
            while let Ok(event) = engine_receiver.try_recv() {
                self.process_engine_event(event, &discard_outbound);
            }
        });
        self.capture = controller;
        stop_result?;
        self.running_notified = false;
        Ok((operation_id, profile))
    }

    fn process_engine_event(&mut self, event: EngineEvent, outbound: &Sender<Value>) {
        let expects_direct_hit_append = matches!(
            &event,
            EngineEvent::Hit(hit) if !hit.is_server_damage_reconciliation()
        );
        let previous_hit_count = self.state.hits.len();
        let previous_hits_generation = self.state.hits_generation;
        let previous_abyss_event_count = self.state.abyss.event_count;
        let time_stop_state_may_change = match &event {
            EngineEvent::TimeStop(
                TimeStopEvent::GamePauseStarted { timestamp, .. }
                | TimeStopEvent::GamePauseMaskChanged { timestamp, .. },
            ) => timestamp.is_finite(),
            EngineEvent::TimeStop(TimeStopEvent::GamePauseEnded { .. }) => {
                self.state.is_game_paused()
            }
            _ => false,
        };
        let signal = apply_engine_event(&mut self.state, event);
        let dropped_hits = if expects_direct_hit_append {
            previous_hit_count
                .saturating_add(1)
                .saturating_sub(self.state.hits.len()) as u64
        } else {
            0
        };
        match signal {
            CoreSignal::Unchanged => {}
            CoreSignal::StateChanged => {
                let battle_projection_changed = self.state.hits_generation
                    != previous_hits_generation
                    || self.state.abyss.event_count != previous_abyss_event_count
                    || time_stop_state_may_change;
                if battle_projection_changed {
                    self.mark_battle_changed(dropped_hits);
                    self.battle_summary_dirty = true;
                }
            }
            CoreSignal::DebugPacket => {}
            CoreSignal::PacketObserved => self.mark_battle_changed(0),
            CoreSignal::CombatClockHealthChanged => self.mark_battle_changed(0),
            CoreSignal::ModScript { state_changed, .. } => {
                if state_changed {
                    self.mark_battle_changed(0);
                    self.battle_summary_dirty = true;
                }
            }
            CoreSignal::InventoryCharactersReplaced => {}
            CoreSignal::InventoryReplaced => self.publish_inventory_snapshot(outbound),
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
                    self.finalize_current_battle_record(Some(&operation_id));
                    self.send_final_battle_summary(outbound);
                    self.send_capture_status_for(outbound, operation_id, profile, "stopped");
                }
            }
        }
    }

    fn battle_summary(&self, subtract_time_stop: bool) -> Option<BattleSummaryDto> {
        self.state
            .session_summary(
                CaptureQualitySource::Live,
                DpsTimeBasis::from_subtract_time_stop(subtract_time_stop),
                false,
            )
            .as_ref()
            .map(BattleSummaryDto::from)
    }

    fn mark_battle_changed(&mut self, dropped_hits: u64) {
        if self.state.hits.is_empty()
            && self.state.stats.is_empty()
            && !self.state.abyss.is_active()
        {
            return;
        }
        let operation_id = self
            .active_operation_id
            .as_ref()
            .or(self.latest_operation_id.as_ref())
            .cloned();
        if self.battle_record.is_none() {
            self.battle_record_sequence = self.battle_record_sequence.saturating_add(1);
            self.battle_record = Some(BattleRecordRuntime {
                id: format!("battle-{}", self.battle_record_sequence),
                capture_operation_id: operation_id.clone(),
                generation: 0,
                finalized: false,
                finalized_at_unix_ms: None,
                axis_base_sequence: 0,
                source: CaptureQualitySource::Live,
            });
        }
        if let Some(record) = self.battle_record.as_mut() {
            if record.capture_operation_id.is_none() || record.finalized {
                record.capture_operation_id.clone_from(&operation_id);
            }
            record.axis_base_sequence = record.axis_base_sequence.saturating_add(dropped_hits);
            record.generation = record.generation.saturating_add(1);
            record.finalized = false;
            record.finalized_at_unix_ms = None;
        }
    }

    fn finalize_current_battle_record(&mut self, operation_id: Option<&str>) {
        let Some(record) = self.battle_record.as_mut() else {
            return;
        };
        if operation_id.is_some()
            && record.capture_operation_id.is_some()
            && operation_id != record.capture_operation_id.as_deref()
        {
            return;
        }
        if !record.finalized {
            record.finalized = true;
            record.finalized_at_unix_ms = Some(unix_time_ms());
            record.generation = record.generation.saturating_add(1);
        }
    }

    fn battle_record_context(
        &self,
        requested_id: Option<&str>,
    ) -> Result<Option<BattleRecordContext<'_>>, BattleReadError> {
        let Some(record) = self.battle_record.as_ref() else {
            return if requested_id.is_some() {
                Err(BattleReadError::RecordNotFound)
            } else {
                Ok(None)
            };
        };
        if requested_id.is_some_and(|requested_id| requested_id != record.id) {
            return Err(BattleReadError::RecordNotFound);
        }
        Ok(Some(record.context()))
    }

    fn battle_record(
        &self,
        params: BattleRecordParams,
    ) -> Result<Option<BattleRecordDto>, BattleReadError> {
        let Some(context) = self.battle_record_context(params.battle_record_id.as_deref())? else {
            return Ok(None);
        };
        Ok(Some(battle_record(
            &self.state,
            context,
            params.subtract_time_stop,
        )))
    }

    fn battle_axis(
        &self,
        params: BattleAxisParams,
    ) -> Result<Option<BattleAxisDto>, BattleReadError> {
        let Some(context) = self.battle_record_context(params.battle_record_id.as_deref())? else {
            return Ok(None);
        };
        battle_axis(&self.state, context, params.cursor, params.limit).map(Some)
    }

    fn battle_timeline(
        &self,
        params: BattleTimelineParams,
    ) -> Result<Option<BattleTimelineDto>, BattleReadError> {
        let Some(context) = self.battle_record_context(params.battle_record_id.as_deref())? else {
            return Ok(None);
        };
        let scope = match params.scope {
            BattleTimelineScopeParam::All => TimelineScope::Whole,
            BattleTimelineScopeParam::Upper => TimelineScope::First,
            BattleTimelineScopeParam::Lower => TimelineScope::Second,
        };
        battle_timeline(
            &self.state,
            &self.characters,
            context,
            scope,
            params.bucket_seconds,
            params.subtract_time_stop,
        )
        .map(Some)
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
        self.battle_record = None;
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
    let (engine_sender, engine_receiver) = bounded(ENGINE_EVENT_QUEUE_CAPACITY);
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
    let mut equipment_response_rx = runtime.mods_plugin.response_receiver();
    loop {
        select! {
            recv(writer_event_rx) -> _ => {
                stop_for_exit(&mut runtime, &engine_receiver, outbound_tx);
                return;
            },
            recv(engine_receiver) -> event => {
                if let Ok(event) = event {
                    runtime.process_engine_event(event, outbound_tx);
                }
            },
            recv(equipment_response_rx) -> response => {
                match response {
                    Ok(response) => {
                        if runtime.process_equipment_response(response, outbound_tx) {
                            return;
                        }
                    }
                    Err(_) => {
                        if runtime.fail_pending_equipment_requests(outbound_tx) {
                            return;
                        }
                        equipment_response_rx = never();
                    }
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
            let _ = outbound.try_send(success(
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
            let result = runtime.stop_capture_with_drain(engine_receiver);
            drain_engine_events(runtime, engine_receiver, outbound);
            match result {
                Ok((operation_id, profile)) => {
                    runtime.finalize_current_battle_record(Some(&operation_id));
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
        Request::Equipment(operation) => {
            let request = mods_plugin_request(runtime.next_equipment_request_id(), operation);
            match runtime.submit_equipment_request(id.clone(), request) {
                Ok(()) => false,
                Err(ModsPluginSubmitError::Busy) => send(
                    outbound,
                    failure(
                        id,
                        RpcError::domain(
                            "MODS_PLUGIN_BUSY",
                            "Mod loader already has the maximum number of pending requests",
                        ),
                    ),
                ),
                Err(ModsPluginSubmitError::Disconnected) => send(
                    outbound,
                    failure(
                        id,
                        RpcError::domain("MODS_PLUGIN_UNAVAILABLE", "Mod loader is unavailable"),
                    ),
                ),
            }
        }
        Request::BattleGetSummary(BattleSummaryParams { subtract_time_stop }) => {
            drain_engine_events_for_battle_read(runtime, engine_receiver, outbound);
            send(
                outbound,
                success(id, runtime.battle_summary(subtract_time_stop)),
            )
        }
        Request::BattleGetRecord(params) => {
            drain_engine_events_for_battle_read(runtime, engine_receiver, outbound);
            let message = match runtime.battle_record(params) {
                Ok(record) => success(id, record),
                Err(error) => failure(id, battle_read_error(error)),
            };
            send(outbound, message)
        }
        Request::BattleGetAxis(params) => {
            drain_engine_events_for_battle_read(runtime, engine_receiver, outbound);
            let message = match runtime.battle_axis(params) {
                Ok(axis) => success(id, axis),
                Err(error) => failure(id, battle_read_error(error)),
            };
            send(outbound, message)
        }
        Request::BattleGetTimeline(params) => {
            drain_engine_events_for_battle_read(runtime, engine_receiver, outbound);
            let message = match runtime.battle_timeline(params) {
                Ok(timeline) => success(id, timeline),
                Err(error) => failure(id, battle_read_error(error)),
            };
            send(outbound, message)
        }
        Request::BattleReset => {
            drain_engine_events(runtime, engine_receiver, outbound);
            runtime.reset_battle();
            send(outbound, success(id, BattleResetResult { reset: true }))
        }
        Request::Unknown => send(outbound, failure(id, RpcError::method_not_found())),
    }
}

fn mods_plugin_request(request_id: u64, operation: EquipmentOperationParam) -> ModsPluginRequest {
    let (character, operation) = match operation {
        EquipmentOperationParam::EquipModule {
            character,
            equipment,
            row,
            column,
        } => (
            item_net_id(character),
            ModsPluginOperation::EquipModule {
                equipment: item_net_id(equipment),
                row,
                column,
            },
        ),
        EquipmentOperationParam::EquipCore {
            character,
            equipment,
        } => (
            item_net_id(character),
            ModsPluginOperation::EquipCore {
                equipment: item_net_id(equipment),
            },
        ),
        EquipmentOperationParam::UnequipModule {
            character,
            equipment,
        } => (
            item_net_id(character),
            ModsPluginOperation::UnequipModule {
                equipment: item_net_id(equipment),
            },
        ),
        EquipmentOperationParam::UnequipCore {
            character,
            equipment,
        } => (
            item_net_id(character),
            ModsPluginOperation::UnequipCore {
                equipment: item_net_id(equipment),
            },
        ),
        EquipmentOperationParam::UnequipAll { character } => {
            (item_net_id(character), ModsPluginOperation::UnequipAll)
        }
        EquipmentOperationParam::EquipOneKey {
            character,
            placements,
            core,
        } => (
            item_net_id(character),
            ModsPluginOperation::EquipOneKey {
                placements: placements
                    .into_iter()
                    .map(|placement| ModsPluginPlacement {
                        equipment: item_net_id(placement.equipment),
                        row: placement.row,
                        column: placement.column,
                    })
                    .collect(),
                core: item_net_id(core),
            },
        ),
        EquipmentOperationParam::MoveModuleToCharacter {
            character,
            equipment,
            row,
            column,
        } => (
            item_net_id(character),
            ModsPluginOperation::MoveModuleToCharacter {
                equipment: item_net_id(equipment),
                row,
                column,
            },
        ),
        EquipmentOperationParam::MoveCoreToCharacter {
            character,
            equipment,
        } => (
            item_net_id(character),
            ModsPluginOperation::MoveCoreToCharacter {
                equipment: item_net_id(equipment),
            },
        ),
        EquipmentOperationParam::SetItemDiscarded {
            equipment,
            discarded,
        } => (
            HtItemNetId::ZERO,
            ModsPluginOperation::SetItemDiscarded {
                equipment: item_net_id(equipment),
                discarded,
            },
        ),
        EquipmentOperationParam::SetItemLocked { equipment, locked } => (
            HtItemNetId::ZERO,
            ModsPluginOperation::SetItemLocked {
                equipment: item_net_id(equipment),
                locked,
            },
        ),
    };
    ModsPluginRequest {
        request_id,
        character,
        operation,
    }
}

fn item_net_id(uid: ItemUidParam) -> HtItemNetId {
    HtItemNetId {
        solt: uid.slot,
        serial: uid.serial,
    }
}

fn stop_for_exit(
    runtime: &mut Runtime,
    engine_receiver: &Receiver<EngineEvent>,
    outbound: &Sender<Value>,
) {
    if runtime.capture.is_running() {
        let (operation_id, profile) = runtime
            .stop_capture_with_drain(engine_receiver)
            .expect("capture checked as running must stop");
        runtime.finalize_current_battle_record(Some(&operation_id));
        // Exit cleanup is best-effort and must finish even when stdout's
        // bounded queue is full. Build at most two terminal notifications in a
        // local bounded lane, then forward them without waiting.
        let (terminal_sender, terminal_receiver) = bounded(2);
        runtime.send_final_battle_summary(&terminal_sender);
        runtime.send_capture_status_for(&terminal_sender, operation_id, profile, "stopped");
        drop(terminal_sender);
        for message in terminal_receiver.try_iter() {
            let _ = outbound.try_send(message);
        }
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

fn drain_engine_events_for_battle_read(
    runtime: &mut Runtime,
    engine_receiver: &Receiver<EngineEvent>,
    outbound: &Sender<Value>,
) {
    if !runtime.capture.is_running() {
        drain_engine_events(runtime, engine_receiver, outbound);
        return;
    }
    for _ in 0..LIVE_BATTLE_READ_EVENT_DRAIN_LIMIT {
        let Ok(event) = engine_receiver.try_recv() else {
            break;
        };
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
        CoreErrorCode::CaptureStateUnavailable => RpcError::domain(
            "CAPTURE_STATE_UNAVAILABLE",
            "The live capture state is unavailable",
        ),
    }
}

fn battle_read_error(error: BattleReadError) -> RpcError {
    match error {
        BattleReadError::RecordNotFound => RpcError::domain(
            "BATTLE_RECORD_NOT_FOUND",
            "The requested battle record is not available in this Core process",
        ),
        BattleReadError::AxisCursorExpired { first_available } => RpcError::domain(
            "BATTLE_AXIS_CURSOR_EXPIRED",
            format!(
                "The requested cursor was trimmed; first available cursor is {first_available}"
            ),
        ),
        BattleReadError::AxisCursorInvalid { last_available } => RpcError::domain(
            "BATTLE_AXIS_CURSOR_INVALID",
            format!("The requested cursor exceeds the last available hit {last_available}"),
        ),
        BattleReadError::TimelineTooLarge => RpcError::domain(
            "BATTLE_TIMELINE_TOO_LARGE",
            "The requested timeline exceeds the bounded response budget; increase bucket_seconds",
        ),
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
    use std::sync::atomic::{AtomicBool, Ordering};

    use crate::engine::model::{
        EmptyCurtainCharacter, EmptyCurtainItem, EmptyCurtainPlacement, Hit, HitCharacterSource,
        HitDirection, HitFollowUp, HtItemNetId, PacketDebug, TimeStopEvent,
    };

    #[test]
    fn engine_event_queue_is_bounded_fifo_and_disconnects_producers() {
        let (sender, receiver) = bounded(ENGINE_EVENT_QUEUE_CAPACITY);
        assert_eq!(sender.capacity(), Some(ENGINE_EVENT_QUEUE_CAPACITY));
        for index in 0..ENGINE_EVENT_QUEUE_CAPACITY {
            sender
                .try_send(EngineEvent::Status(index.to_string()))
                .expect("queue has declared capacity");
        }
        assert!(matches!(
            sender.try_send(EngineEvent::CaptureStopped),
            Err(crossbeam_channel::TrySendError::Full(_))
        ));
        assert!(matches!(
            receiver.recv().expect("first queued event"),
            EngineEvent::Status(status) if status == "0"
        ));
        sender
            .try_send(EngineEvent::CaptureStopped)
            .expect("released capacity accepts the next reliable event");
        drop(receiver);
        assert!(matches!(
            sender.try_send(EngineEvent::CaptureStopped),
            Err(crossbeam_channel::TrySendError::Disconnected(_))
        ));
    }

    #[test]
    fn capture_stop_drains_a_full_engine_queue_before_join_without_outbound_io() {
        let resources = RuntimeResources::load().expect("runtime resources");
        let (engine_sender, engine_receiver) = bounded(1);
        engine_sender
            .send(EngineEvent::Hit(Box::new(test_hit(1.0, 100.0))))
            .expect("prefill engine queue");
        let stop = Arc::new(AtomicBool::new(false));
        let producer_stop = Arc::clone(&stop);
        let sink = crate::engine::capture::EngineEventSink::reliable(engine_sender.clone());
        let producer = thread::spawn(move || {
            while !producer_stop.load(Ordering::Relaxed) {
                thread::yield_now();
            }
            if sink
                .send(EngineEvent::Hit(Box::new(test_hit(2.0, 200.0))))
                .is_ok()
            {
                let _ = sink.send(EngineEvent::CaptureStopped);
            }
        });
        let capture = crate::engine::capture::CaptureHandle::from_test_thread(stop, producer);
        let (latest_battle, _) = latest_message_channel();
        let mut runtime = Runtime::new(
            resources,
            engine_sender,
            latest_battle,
            PathBuf::from("logs"),
        );
        runtime
            .capture
            .install_test_capture(capture, CaptureProfile::Combat);
        runtime.active_operation_id = Some("capture-test".to_owned());
        runtime.latest_operation_id = runtime.active_operation_id.clone();

        let (completed_sender, completed_receiver) = bounded(1);
        thread::spawn(move || {
            let result = runtime.stop_capture_with_drain(&engine_receiver);
            let _ = completed_sender.send((
                result.is_ok(),
                runtime.state.total_damage,
                runtime.capture.is_running(),
            ));
        });
        let (stopped, total_damage, still_running) = completed_receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("full engine queue stop must finish within its cancellation bound");
        assert!(stopped);
        assert_eq!(
            total_damage, 300.0,
            "FIFO semantic events must not be dropped"
        );
        assert!(!still_running);
    }

    #[test]
    fn shutdown_finishes_with_full_engine_and_outbound_queues() {
        let resources = RuntimeResources::load().expect("runtime resources");
        let (engine_sender, engine_receiver) = bounded(1);
        engine_sender
            .send(EngineEvent::Hit(Box::new(test_hit(1.0, 100.0))))
            .expect("prefill engine queue");
        let stop = Arc::new(AtomicBool::new(false));
        let producer_stop = Arc::clone(&stop);
        let sink = crate::engine::capture::EngineEventSink::reliable(engine_sender.clone());
        let producer = thread::spawn(move || {
            while !producer_stop.load(Ordering::Relaxed) {
                thread::yield_now();
            }
            if sink
                .send(EngineEvent::Hit(Box::new(test_hit(2.0, 200.0))))
                .is_ok()
            {
                let _ = sink.send(EngineEvent::CaptureStopped);
            }
        });
        let capture = crate::engine::capture::CaptureHandle::from_test_thread(stop, producer);
        let (latest_battle, _) = latest_message_channel();
        let mut runtime = Runtime::new(
            resources,
            engine_sender,
            latest_battle,
            PathBuf::from("logs"),
        );
        runtime.handshaken = true;
        runtime
            .capture
            .install_test_capture(capture, CaptureProfile::Combat);
        runtime.active_operation_id = Some("capture-shutdown".to_owned());
        runtime.latest_operation_id = runtime.active_operation_id.clone();

        let (command_sender, command_receiver) = bounded(1);
        command_sender
            .send(ReaderEvent::Request(ValidatedRequest {
                id: serde_json::json!(1),
                request: Request::Shutdown,
            }))
            .expect("queue shutdown request");
        let (outbound_sender, outbound_receiver) = bounded(1);
        outbound_sender
            .send(serde_json::json!({"blocked": true}))
            .expect("prefill stdout queue");
        let (_writer_event_sender, writer_event_receiver) = unbounded();
        let (completed_sender, completed_receiver) = bounded(1);
        thread::spawn(move || {
            core_loop(
                command_receiver,
                &outbound_sender,
                writer_event_receiver,
                engine_receiver,
                runtime,
            );
            let _ = completed_sender.send(());
        });

        completed_receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("shutdown must not wait on full engine/stdout queues");
        assert_eq!(
            outbound_receiver.try_recv().expect("prefilled stdout item"),
            serde_json::json!({"blocked": true})
        );
    }

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
            .send(EngineEvent::EmptyCurtainCharacters(vec![
                EmptyCurtainCharacter {
                    net_id: HtItemNetId { solt: 6, serial: 7 },
                    character_id: 1020,
                },
                EmptyCurtainCharacter {
                    net_id: HtItemNetId { solt: 8, serial: 9 },
                    character_id: 1075,
                },
            ]))
            .unwrap();
        engine_sender
            .send(EngineEvent::EmptyCurtain(vec![EmptyCurtainItem {
                id: HtItemNetId { solt: 4, serial: 5 },
                item_id: "cell2_style2_1_Orange".to_owned(),
                level: 20,
                main_stats: Vec::new(),
                sub_stats: Vec::new(),
                locked: true,
                discarded: false,
                character_net_id: Some(HtItemNetId { solt: 6, serial: 7 }),
                equipped_character_id: Some(1020),
                equipped_placement: Some(EmptyCurtainPlacement { row: 2, column: 3 }),
            }]))
            .unwrap();
        drain_engine_events(&mut runtime, &engine_receiver, &outbound);
        let inventory = receiver.recv().unwrap();
        assert_eq!(inventory["method"], "event.inventory.snapshot");
        assert_eq!(inventory["params"]["generation"], 1);
        assert_eq!(inventory["params"]["character_count"], 2);
        assert_eq!(
            inventory["params"]["characters"][0],
            serde_json::json!({
                "uid": {"slot": 6, "serial": 7},
                "character_id": 1020
            })
        );
        assert_eq!(
            inventory["params"]["characters"][1],
            serde_json::json!({
                "uid": {"slot": 8, "serial": 9},
                "character_id": 1075
            })
        );
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
            inventory["params"]["items"][0]["equipped_placement"],
            serde_json::json!({"row": 2, "column": 3})
        );
        assert_eq!(
            inventory["params"]["items"][0]["names"]["en"],
            "Type II Module"
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
            EngineEvent::TimeStop(TimeStopEvent::GamePauseStarted {
                timestamp: 2.0,
                pause_type_mask: 1 << 2,
            }),
            &outbound,
        );
        runtime.process_engine_event(
            EngineEvent::TimeStop(TimeStopEvent::GamePauseEnded {
                timestamp: 4.0,
                pause_type_mask: 1 << 2,
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
                discarded: false,
                character_net_id: None,
                equipped_character_id: None,
                equipped_placement: None,
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
    fn live_battle_record_bounds_event_catch_up_before_responding() {
        let resources = RuntimeResources::load().expect("runtime resources");
        let (engine_sender, engine_receiver) = unbounded();
        let (latest_battle, _) = latest_message_channel();
        let mut runtime = Runtime::new(
            resources,
            engine_sender.clone(),
            latest_battle,
            PathBuf::from("logs"),
        );
        runtime.handshaken = true;
        runtime.active_operation_id = Some("capture-live-read".to_owned());
        runtime.latest_operation_id = runtime.active_operation_id.clone();
        runtime.running_notified = true;

        let stop = Arc::new(AtomicBool::new(false));
        let producer_stop = Arc::clone(&stop);
        let sink = crate::engine::capture::EngineEventSink::reliable(engine_sender.clone());
        let producer = thread::spawn(move || {
            while !producer_stop.load(Ordering::Relaxed) {
                thread::yield_now();
            }
            let _ = sink.send(EngineEvent::CaptureStopped);
        });
        runtime.capture.install_test_capture(
            crate::engine::capture::CaptureHandle::from_test_thread(stop, producer),
            CaptureProfile::Combat,
        );

        let (outbound, receiver) = bounded(4);
        runtime.process_engine_event(EngineEvent::Hit(Box::new(test_hit(1.0, 100.0))), &outbound);
        for index in 0..=LIVE_BATTLE_READ_EVENT_DRAIN_LIMIT {
            engine_sender
                .send(EngineEvent::Status(index.to_string()))
                .expect("queue live event");
        }

        assert!(!handle_request(
            ValidatedRequest {
                id: serde_json::json!(3),
                request: Request::BattleGetRecord(BattleRecordParams {
                    battle_record_id: None,
                    subtract_time_stop: true,
                }),
            },
            &mut runtime,
            &engine_receiver,
            &outbound,
        ));

        let record = receiver.recv().expect("record response");
        assert_eq!(record["result"]["summary"]["total_damage"], 100.0);
        assert_eq!(engine_receiver.len(), 1);
        runtime
            .stop_capture_with_drain(&engine_receiver)
            .expect("stop test capture");
    }

    #[test]
    fn equipment_requests_map_external_uids_and_boolean_state() {
        let request = mods_plugin_request(
            17,
            EquipmentOperationParam::MoveModuleToCharacter {
                character: ItemUidParam { slot: 1, serial: 2 },
                equipment: ItemUidParam { slot: 3, serial: 4 },
                row: 2,
                column: 5,
            },
        );
        assert_eq!(request.request_id, 17);
        assert_eq!(request.character, HtItemNetId { solt: 1, serial: 2 });
        assert!(matches!(
            request.operation,
            ModsPluginOperation::MoveModuleToCharacter {
                equipment: HtItemNetId { solt: 3, serial: 4 },
                row: 2,
                column: 5,
            }
        ));

        let request = mods_plugin_request(
            18,
            EquipmentOperationParam::SetItemLocked {
                equipment: ItemUidParam { slot: 5, serial: 6 },
                locked: true,
            },
        );
        assert_eq!(request.character, HtItemNetId::ZERO);
        assert!(matches!(
            request.operation,
            ModsPluginOperation::SetItemLocked {
                equipment: HtItemNetId { solt: 5, serial: 6 },
                locked: true,
            }
        ));
    }

    #[test]
    fn battle_read_methods_are_registered_before_data_exists() {
        let input = br#"{"jsonrpc":"2.0","id":"hello","method":"core.hello","params":{"client_name":"test","client_version":"1","protocol_min":1,"protocol_max":1}}
{"jsonrpc":"2.0","id":"record","method":"battle.get_record","params":{"subtract_time_stop":true}}
{"jsonrpc":"2.0","id":"axis","method":"battle.get_axis","params":{"cursor":null,"limit":250}}
{"jsonrpc":"2.0","id":"timeline","method":"battle.get_timeline","params":{"scope":"all","bucket_seconds":1.0,"subtract_time_stop":true}}
{"jsonrpc":"2.0","id":"shutdown","method":"core.shutdown","params":{}}
"#;
        let output = SharedWriter::default();
        let captured = output.clone();

        assert_eq!(run(Cursor::new(input), output, PathBuf::from("logs")), 0);

        let lines = String::from_utf8(captured.bytes())
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();
        for id in ["record", "axis", "timeline"] {
            let response = lines
                .iter()
                .find(|line| line["id"] == id)
                .expect("battle read response");
            assert_eq!(response["result"], Value::Null);
            assert!(response.get("error").is_none());
        }
    }

    #[test]
    fn battle_read_models_share_a_stable_record_lifecycle_and_revision() {
        let resources = RuntimeResources::load().expect("runtime resources");
        let (engine_sender, _) = unbounded();
        let (latest_battle, _) = latest_message_channel();
        let mut runtime = Runtime::new(
            resources,
            engine_sender,
            latest_battle,
            PathBuf::from("logs"),
        );
        let (outbound, _) = bounded(8);
        runtime.active_operation_id = Some("capture-9".to_owned());

        runtime.process_engine_event(EngineEvent::Hit(Box::new(test_hit(1.0, 100.0))), &outbound);
        runtime.process_engine_event(EngineEvent::Hit(Box::new(test_hit(2.0, 200.0))), &outbound);

        let record = runtime
            .battle_record(BattleRecordParams {
                battle_record_id: None,
                subtract_time_stop: true,
            })
            .expect("record query")
            .expect("record exists");
        assert_eq!(record.battle_record_id, "battle-1");
        assert_eq!(record.capture_operation_id.as_deref(), Some("capture-9"));
        assert_eq!(record.state, "live");
        let live_generation = record.generation.parse::<u64>().expect("generation");

        let axis = runtime
            .battle_axis(BattleAxisParams {
                battle_record_id: Some(record.battle_record_id.clone()),
                cursor: None,
                limit: 1,
            })
            .expect("axis query")
            .expect("axis exists");
        assert_eq!(axis.generation, record.generation);
        assert_eq!(axis.rows.len(), 1);
        assert_eq!(axis.next_cursor.as_deref(), Some("2"));

        let timeline = runtime
            .battle_timeline(BattleTimelineParams {
                battle_record_id: Some(record.battle_record_id.clone()),
                scope: BattleTimelineScopeParam::All,
                bucket_seconds: 1.0,
                subtract_time_stop: true,
            })
            .expect("timeline query")
            .expect("timeline exists");
        assert_eq!(timeline.generation, record.generation);
        assert_eq!(timeline.total_damage, 300.0);
        assert!(!timeline.buckets.is_empty());

        runtime.process_engine_event(EngineEvent::EmptyCurtainCharacters(Vec::new()), &outbound);
        assert_eq!(
            runtime.battle_record.as_ref().expect("record").generation,
            live_generation,
            "inventory-only changes must not advance the battle read generation"
        );
        runtime.process_engine_event(
            EngineEvent::HitFollowUp(HitFollowUp {
                source_byte_offset: None,
                source_bit_shift: None,
                source_target_id: None,
                source_timestamp: 99.0,
                source_char_id: 999,
                source_damage: 1.0,
                source_target_hp_before: 0.0,
                source_target_hp_after: 0.0,
                source_target_max_hp: 0.0,
                source_gameplay_effect_index: None,
                timestamp: 100.0,
                damage: 1.0,
                target_hp_after: 0.0,
                target_hp_percent: 0.0,
                damage_name: None,
                attack_type: None,
                damage_attribute: None,
            }),
            &outbound,
        );
        assert_eq!(
            runtime.battle_record.as_ref().expect("record").generation,
            live_generation,
            "a no-op follow-up must not advance the battle read generation"
        );
        runtime.process_engine_event(
            EngineEvent::TimeStop(TimeStopEvent::GamePauseEnded {
                timestamp: 101.0,
                pause_type_mask: 1,
            }),
            &outbound,
        );
        assert_eq!(
            runtime.battle_record.as_ref().expect("record").generation,
            live_generation,
            "an unmatched pause end must not advance the battle read generation"
        );

        runtime.finalize_current_battle_record(Some("capture-9"));
        let finalized = runtime
            .battle_record(BattleRecordParams {
                battle_record_id: Some(record.battle_record_id.clone()),
                subtract_time_stop: true,
            })
            .expect("final record query")
            .expect("record exists");
        assert_eq!(finalized.state, "finalized");
        assert_eq!(
            finalized.generation.parse::<u64>().expect("generation"),
            live_generation + 1
        );
        assert!(finalized.finalized_at_unix_ms.is_some());

        runtime.reset_battle();
        assert!(
            runtime
                .battle_record(BattleRecordParams {
                    battle_record_id: None,
                    subtract_time_stop: true,
                })
                .expect("empty record query")
                .is_none()
        );
        assert!(matches!(
            runtime.battle_record(BattleRecordParams {
                battle_record_id: Some(record.battle_record_id),
                subtract_time_stop: true,
            }),
            Err(BattleReadError::RecordNotFound)
        ));

        runtime.process_engine_event(EngineEvent::Hit(Box::new(test_hit(3.0, 50.0))), &outbound);
        assert_eq!(
            runtime.battle_record.as_ref().expect("new record").id,
            "battle-2"
        );
    }

    #[test]
    fn server_damage_reconciliation_in_place_keeps_the_cli_axis_complete() {
        let resources = RuntimeResources::load().expect("runtime resources");
        let (engine_sender, _) = unbounded();
        let (latest_battle, _) = latest_message_channel();
        let mut runtime = Runtime::new(
            resources,
            engine_sender,
            latest_battle,
            PathBuf::from("logs"),
        );
        let (outbound, _) = bounded(4);

        runtime.process_engine_event(
            EngineEvent::Hit(Box::new(targeted_test_hit(105.0))),
            &outbound,
        );
        runtime.process_engine_event(
            EngineEvent::Hit(Box::new(server_reconciliation_marker(99.0))),
            &outbound,
        );

        let record = runtime
            .battle_record(BattleRecordParams {
                battle_record_id: None,
                subtract_time_stop: true,
            })
            .expect("record query")
            .expect("record exists");
        let axis = runtime
            .battle_axis(BattleAxisParams {
                battle_record_id: Some(record.battle_record_id.clone()),
                cursor: None,
                limit: 10,
            })
            .expect("axis query")
            .expect("axis exists");

        assert!(record.axis_complete);
        assert!(axis.complete);
        assert_eq!(axis.first_available_cursor, "1");
        assert_eq!(axis.total_hits, "1");
        assert_eq!(axis.rows.len(), 1);
        assert_eq!(runtime.state.hits[0].overkill_damage(), 6.0);
    }

    #[test]
    fn positive_server_damage_residual_appends_without_trimming_the_cli_axis() {
        let resources = RuntimeResources::load().expect("runtime resources");
        let (engine_sender, _) = unbounded();
        let (latest_battle, _) = latest_message_channel();
        let mut runtime = Runtime::new(
            resources,
            engine_sender,
            latest_battle,
            PathBuf::from("logs"),
        );
        let (outbound, _) = bounded(4);

        runtime.process_engine_event(
            EngineEvent::Hit(Box::new(targeted_test_hit(80.0))),
            &outbound,
        );
        runtime.process_engine_event(
            EngineEvent::Hit(Box::new(server_reconciliation_marker(99.0))),
            &outbound,
        );

        let record = runtime
            .battle_record(BattleRecordParams {
                battle_record_id: None,
                subtract_time_stop: true,
            })
            .expect("record query")
            .expect("record exists");
        let axis = runtime
            .battle_axis(BattleAxisParams {
                battle_record_id: Some(record.battle_record_id.clone()),
                cursor: None,
                limit: 10,
            })
            .expect("axis query")
            .expect("axis exists");

        assert!(record.axis_complete);
        assert!(axis.complete);
        assert_eq!(axis.first_available_cursor, "1");
        assert_eq!(axis.total_hits, "2");
        assert_eq!(axis.rows.len(), 2);
        assert_eq!(runtime.state.total_damage, 99.0);
    }

    #[test]
    fn unknown_and_duplicate_mod_responses_do_not_consume_pending_requests_or_stop_runtime() {
        let resources = RuntimeResources::load().expect("runtime resources");
        let (engine_sender, _) = unbounded();
        let (latest_battle, _) = latest_message_channel();
        let mut runtime = Runtime::new(
            resources,
            engine_sender,
            latest_battle,
            PathBuf::from("logs"),
        );
        runtime
            .pending_equipment_requests
            .insert(1, serde_json::json!("rpc-1"));
        let (outbound, receiver) = unbounded();

        assert!(!runtime.process_equipment_response(
            ModsPluginResponse {
                request_id: 99,
                status: Ok(0),
            },
            &outbound,
        ));
        assert_eq!(
            runtime.pending_equipment_requests.get(&1),
            Some(&serde_json::json!("rpc-1"))
        );
        assert!(receiver.try_recv().is_err());

        assert!(!runtime.process_equipment_response(
            ModsPluginResponse {
                request_id: 1,
                status: Ok(0),
            },
            &outbound,
        ));
        assert_eq!(receiver.recv().expect("normal response")["id"], "rpc-1");
        assert!(!runtime.process_equipment_response(
            ModsPluginResponse {
                request_id: 1,
                status: Ok(0),
            },
            &outbound,
        ));
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn disconnected_mod_loader_fails_pending_equipment_requests() {
        let resources = RuntimeResources::load().expect("runtime resources");
        let (engine_sender, _) = unbounded();
        let (latest_battle, _) = latest_message_channel();
        let mut runtime = Runtime::new(
            resources,
            engine_sender,
            latest_battle,
            PathBuf::from("logs"),
        );
        runtime
            .pending_equipment_requests
            .insert(1, serde_json::json!("rpc-1"));
        runtime
            .pending_equipment_requests
            .insert(2, serde_json::json!("rpc-2"));
        let (outbound, receiver) = unbounded();

        assert!(!runtime.fail_pending_equipment_requests(&outbound));
        assert!(runtime.pending_equipment_requests.is_empty());
        let messages = [
            receiver.recv().expect("first unavailable response"),
            receiver.recv().expect("second unavailable response"),
        ];
        assert!(messages.iter().all(|message| {
            message["error"]["data"]["domain_code"] == "MODS_PLUGIN_UNAVAILABLE"
        }));
        let mut ids = [
            messages[0]["id"]
                .as_str()
                .expect("first response ID")
                .to_owned(),
            messages[1]["id"]
                .as_str()
                .expect("second response ID")
                .to_owned(),
        ];
        ids.sort();
        assert_eq!(ids, ["rpc-1", "rpc-2"]);
    }

    #[test]
    fn equipment_pipe_call_does_not_block_core_loop() {
        let resources = RuntimeResources::load().unwrap();
        let (engine_sender, engine_receiver) = unbounded();
        let (latest_battle, _) = latest_message_channel();
        let mut runtime = Runtime::new(
            resources,
            engine_sender,
            latest_battle,
            PathBuf::from("logs"),
        );
        runtime.handshaken = true;
        let (started_tx, started_rx) = bounded(1);
        let (release_tx, release_rx) = bounded(1);
        let call_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let worker_call_count = Arc::clone(&call_count);
        runtime.mods_plugin = ModsPluginClient::with_call_for_test(move |_| {
            if worker_call_count.fetch_add(1, std::sync::atomic::Ordering::AcqRel) == 0 {
                started_tx.send(()).unwrap();
                release_rx
                    .recv()
                    .map_err(|_| "test pipe call release channel closed".to_owned())?;
            }
            Ok(0)
        });

        let (command_tx, command_rx) = bounded(4);
        let (outbound_tx, outbound_rx) = bounded(4);
        let (writer_event_tx, writer_event_rx) = unbounded();
        let core_outbound = outbound_tx.clone();
        let core = thread::spawn(move || {
            core_loop(
                command_rx,
                &core_outbound,
                writer_event_rx,
                engine_receiver,
                runtime,
            );
        });
        command_tx
            .send(ReaderEvent::Request(ValidatedRequest {
                id: serde_json::json!(1),
                request: Request::Equipment(EquipmentOperationParam::SetItemLocked {
                    equipment: ItemUidParam { slot: 3, serial: 4 },
                    locked: true,
                }),
            }))
            .unwrap();
        started_rx.recv_timeout(Duration::from_secs(1)).unwrap();

        command_tx
            .send(ReaderEvent::Request(ValidatedRequest {
                id: serde_json::json!(2),
                request: Request::Equipment(EquipmentOperationParam::SetItemLocked {
                    equipment: ItemUidParam { slot: 5, serial: 6 },
                    locked: true,
                }),
            }))
            .unwrap();
        command_tx
            .send(ReaderEvent::Request(ValidatedRequest {
                id: serde_json::json!(3),
                request: Request::Equipment(EquipmentOperationParam::SetItemLocked {
                    equipment: ItemUidParam { slot: 7, serial: 8 },
                    locked: true,
                }),
            }))
            .unwrap();
        command_tx
            .send(ReaderEvent::Request(ValidatedRequest {
                id: serde_json::json!(4),
                request: Request::Status,
            }))
            .unwrap();
        let busy = outbound_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(busy["id"], 3);
        assert_eq!(busy["error"]["data"]["domain_code"], "MODS_PLUGIN_BUSY");
        let status = outbound_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(status["id"], 4);
        assert!(status.get("result").is_some());

        release_tx.send(()).unwrap();
        for expected_id in [1, 2] {
            let equipment = outbound_rx.recv_timeout(Duration::from_secs(1)).unwrap();
            assert_eq!(equipment["id"], expected_id);
            assert_eq!(equipment["result"]["status"], "rpc_dispatched");
        }

        command_tx.send(ReaderEvent::Eof).unwrap();
        core.join().unwrap();
        drop(writer_event_tx);
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

    #[test]
    fn poisoned_latest_message_slot_drops_partial_value_and_recovers_empty() {
        let slot = Mutex::new(Some(serde_json::json!({"partial": true})));
        let _ = std::panic::catch_unwind(|| {
            let mut guard = slot.lock().expect("test latest-message slot");
            *guard = Some(serde_json::json!({"leaked": true}));
            panic!("poison test latest-message slot");
        });

        let mut guard = lock_latest_message_slot(&slot);
        assert!(guard.is_none());
        *guard = Some(serde_json::json!({"generation": 2}));
        drop(guard);
        assert!(!slot.is_poisoned());
        assert_eq!(
            lock_latest_message_slot(&slot).take(),
            Some(serde_json::json!({"generation": 2}))
        );
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
            damage_name: Some("Skill".to_owned()),
            damage_component: None,
            attack_type: Some("normal".to_owned()),
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

    fn targeted_test_hit(damage: f64) -> Hit {
        let mut hit = test_hit(1.0, damage);
        hit.target_id = Some("enemy-wire:test".to_owned());
        hit.target_hp_before = 100.0;
        hit.target_hp_after = (100.0 - damage).max(0.0);
        hit.target_max_hp = 100.0;
        hit.reconciled_overkill_damage = Some(0.0);
        hit
    }

    fn server_reconciliation_marker(authoritative_damage: f64) -> Hit {
        let mut hit = test_hit(1.1, 0.0);
        hit.char_id = 0;
        hit.char_name = "Unattributed".to_owned();
        hit.char_known = false;
        hit.target_id = Some("enemy-wire:test".to_owned());
        hit.target_hp_before = authoritative_damage;
        hit.target_hp_after = 0.0;
        hit.target_max_hp = 100.0;
        hit.damage_name = Some("Server settlement residual".to_owned());
        hit.reconciled_overkill_damage = Some(0.0);
        hit
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
#[test]
fn capture_state_unavailable_has_a_stable_private_detail_free_rpc_error() {
    let error = core_error(CoreErrorCode::CaptureStateUnavailable);

    assert_eq!(error.message, "Core error");
    let data = error.data.expect("domain error data");
    assert_eq!(data.domain_code, "CAPTURE_STATE_UNAVAILABLE");
    assert_eq!(data.detail, "The live capture state is unavailable");
    assert!(!data.detail.contains("poison"));
    assert!(!data.detail.contains("mutex"));
}
