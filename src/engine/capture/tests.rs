fn parse_capture_export(text: impl Into<String>) -> Result<CaptureExportDocument, String> {
    parse_capture_export_typed(text).map_err(|error| error.to_string())
}

use super::*;

use std::time::{SystemTime, UNIX_EPOCH};

use crossbeam_channel::unbounded;
use pcap_file::pcapng::blocks::unknown::UnknownBlock;
impl CaptureHandle {
    pub(crate) fn from_test_thread(stop: Arc<AtomicBool>, thread: thread::JoinHandle<()>) -> Self {
        Self {
            stop,
            thread: Some(thread),
            raw_capture: RawCaptureBuffer::new(None),
        }
    }
}

impl ServerDamageCalibrationTracker {
    fn observe_hit(&mut self, hit: &Hit) {
        let _ = self.observe_hit_with_semantics(hit, false, 0);
    }
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
        let observation =
            self.observe_server_damage_settlements_with_sources(timestamp, settlements);
        (observation.corrections, observation.unattributed)
    }
    fn observe_boss_hp(
        &mut self,
        timestamp: f64,
        update: &crate::engine::parser::ParsedBossHpUpdate,
    ) -> Option<HitDamageCorrection> {
        self.observe_boss_hp_detailed(timestamp, update).0
    }
}

impl PacketDecoder {
    fn with_server_damage_calibration(use_server_damage_calibration: bool) -> Self {
        Self {
            use_server_damage_calibration,
            ..Self::default()
        }
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
        let reconciliation =
            self.reconcile_server_damage_settlements_with_sources(timestamp, settlements);
        (
            reconciliation.corrections,
            reconciliation.unattributed,
            reconciliation.residual_hits,
        )
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
        let reconciliation = self.reconcile_current_packet_server_damage_settlements_with_sources(
            timestamp,
            settlements,
            hits,
        );
        (
            reconciliation.corrections,
            reconciliation.unattributed,
            reconciliation.residual_hits,
        )
    }
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

fn current_filetime_100ns() -> u64 {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time must be after the Unix epoch");
    FILETIME_UNIX_EPOCH_100NS
        + elapsed.as_secs() * FILETIME_TICKS_PER_SECOND
        + u64::from(elapsed.subsec_nanos()) / 100
}

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
fn bounded_utf8_reader_rejects_declared_size_and_growth_past_limit() {
    let limit = 8_u64;
    assert!(matches!(
        read_bounded_utf8(std::io::empty(), limit + 1, limit),
        Err(CaptureImportError::TooLarge { size: 9, limit: 8 })
    ));
    let reader = std::io::Cursor::new(b"123456789".to_vec());
    assert!(matches!(
        read_bounded_utf8(reader, limit, limit),
        Err(CaptureImportError::TooLarge { size: 9, limit: 8 })
    ));
}

#[test]
fn combat_clock_health_is_typed_and_published_only_on_change() {
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
        CombatClockRuntimeHealth::DataUnavailable,
    )
    .expect("first degradation is published");
    publish_combat_clock_health(
        &sink,
        &mut previous,
        CombatClockRuntimeHealth::DataUnavailable,
    )
    .expect("duplicate degradation is a no-op");
    assert!(matches!(
        receiver.try_recv(),
        Ok(EngineEvent::CombatClockHealth(
            CombatClockRuntimeHealth::DataUnavailable
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
    assert!(validate_pcapng_import(&file).is_ok());
    assert!(matches!(
        validate_pcapng_import(&directory),
        Err(PcapngImportError::NotAFile)
    ));

    let mut packet_count = 0;
    assert!(matches!(
        account_pcapng_frame(CAPTURE_SNAPLEN as usize + 1, &mut packet_count),
        Err(PcapngImportError::FrameTooLarge { .. })
    ));

    packet_count = MAX_PCAPNG_IMPORT_PACKETS;
    assert!(matches!(
        account_pcapng_frame(0, &mut packet_count),
        Err(PcapngImportError::TooManyPackets { .. })
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
    assert!(
        validate_capture_export_structure(&document).is_ok(),
        "structure within every budget is accepted"
    );
    let mut exact_text_limit = document.clone();
    exact_text_limit.hits[0].char_name = "a".repeat(MAX_CAPTURE_JSON_IMPORT_CHARACTER_NAME_BYTES);
    assert!(
        validate_capture_export_structure(&exact_text_limit).is_ok(),
        "a string exactly at its UTF-8 byte limit is accepted"
    );
    let mut oversized_text = document.clone();
    oversized_text.hits[0].char_name = "界".repeat(43);
    assert!(matches!(
        validate_capture_export_structure(&oversized_text),
        Err(CaptureImportError::FieldTooLarge {
            field: "hits[].char_name",
            size: 129,
            limit: MAX_CAPTURE_JSON_IMPORT_CHARACTER_NAME_BYTES,
        })
    ));
    let mut oversized_item = document.clone();
    oversized_item.empty_curtain[0].item_id = "i".repeat(MAX_CAPTURE_JSON_IMPORT_LABEL_BYTES + 1);
    assert!(matches!(
        validate_capture_export_structure(&oversized_item),
        Err(CaptureImportError::FieldTooLarge {
            field: "empty_curtain[].item_id",
            ..
        })
    ));
    let mut invalid_timestamp = document.clone();
    invalid_timestamp.hits[0].timestamp_unix = MAX_CAPTURE_JSON_IMPORT_TIMESTAMP + 1.0;
    assert!(matches!(
        validate_capture_export_structure(&invalid_timestamp),
        Err(CaptureImportError::InvalidNumber {
            field: "hits[].timestamp_unix"
        })
    ));
    let mut invalid_damage = document.clone();
    invalid_damage.hits[0].damage = -1.0;
    assert!(matches!(
        validate_capture_export_structure(&invalid_damage),
        Err(CaptureImportError::InvalidNumber {
            field: "hits[].damage"
        })
    ));
    type Grow = fn(&mut CaptureExportDocument);
    let over_budget: [(&str, Grow); 7] = [
        ("party", |document| {
            document.party =
                vec![document.party[0].clone(); MAX_CAPTURE_JSON_IMPORT_PARTY_ROWS + 1];
        }),
        ("abyss.first_half.party", |document| {
            let row = document.abyss.first_half.party[0].clone();
            document.abyss.first_half.party = vec![row; MAX_CAPTURE_JSON_IMPORT_PARTY_ROWS + 1];
        }),
        ("empty_curtain", |document| {
            let item = document.empty_curtain[0].clone();
            document.empty_curtain = vec![item; MAX_CAPTURE_JSON_IMPORT_EMPTY_CURTAIN_ITEMS + 1];
        }),
        ("empty_curtain_characters", |document| {
            let character = document.empty_curtain_characters[0];
            document.empty_curtain_characters =
                vec![character; MAX_CAPTURE_JSON_IMPORT_EMPTY_CURTAIN_CHARACTERS + 1];
        }),
        ("time_stop_events", |document| {
            let event = document.time_stop_events[0].clone();
            document.time_stop_events = vec![event; MAX_CAPTURE_JSON_IMPORT_TIME_STOP_EVENTS + 1];
        }),
        ("empty_curtain[].main_stats", |document| {
            let item = &mut document.empty_curtain[0];
            item.main_stats =
                vec![item.main_stats[0].clone(); MAX_CAPTURE_JSON_IMPORT_ITEM_STATS + 1];
        }),
        ("hits[].target_context", |document| {
            document.hits[0].target_context =
                vec!["boss".to_owned(); MAX_CAPTURE_JSON_IMPORT_TARGET_CONTEXT + 1];
        }),
    ];
    for (field, grow) in over_budget {
        let mut oversized = document.clone();
        grow(&mut oversized);
        assert!(matches!(
            validate_capture_export_structure(&oversized),
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
        prepare_capture_json_replay(&exact).is_ok(),
        "a minimal valid document must be prepared"
    );

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
    std::fs::write(&unsupported, br#"{"version":2}"#).expect("write unsupported-version fixture");
    assert!(matches!(
        prepare_capture_json_replay(&unsupported),
        Err(CaptureImportError::UnsupportedVersion {
            found: 2,
            expected: CAPTURE_EXPORT_VERSION
        })
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
    let oversize = forward_capture_frame(&sender, CaptureFrame::new(vec![0; 3], 1.0)).unwrap_err();
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
fn combat_clock_block_round_trips_linko_pause_state() {
    let transition = CombatClockTransitionSnapshot {
        sequence: 9,
        timestamp_100ns: FILETIME_UNIX_EPOCH_100NS + 87 * FILETIME_TICKS_PER_SECOND,
        pause_type_mask: 1 << 6,
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
    let payload = encode_mod_script_block(&event).expect("bounded ModScript event should encode");

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
fn capture_export_accepts_saved_linko_mask_changes() {
    let document: CaptureExportDocument = serde_json::from_str(
        r#"{
            "time_stop_events": [
                {"GamePauseStarted":{"timestamp":10.0,"pause_type_mask":64}},
                {"GamePauseMaskChanged":{"timestamp":11.0,"pause_type_mask":68}},
                {"GamePauseMaskChanged":{"timestamp":12.0,"pause_type_mask":4}},
                {"GamePauseEnded":{"timestamp":13.0,"pause_type_mask":4}}
            ]
        }"#,
    )
    .expect("Linko and Q pause masks should be accepted");

    assert_eq!(
        document.time_stop_events,
        vec![
            TimeStopEvent::GamePauseStarted {
                timestamp: 10.0,
                pause_type_mask: 1 << 6,
            },
            TimeStopEvent::GamePauseMaskChanged {
                timestamp: 11.0,
                pause_type_mask: (1 << 6) | (1 << 2),
            },
            TimeStopEvent::GamePauseMaskChanged {
                timestamp: 12.0,
                pause_type_mask: 1 << 2,
            },
            TimeStopEvent::GamePauseEnded {
                timestamp: 13.0,
                pause_type_mask: 1 << 2,
            },
        ]
    );
}

#[test]
fn game_pause_tracker_preserves_nonzero_mask_changes() {
    let mut tracker = GamePauseIntervalTracker::default();
    assert_eq!(
        tracker.apply_transition(10.0, 1 << 2),
        Some(TimeStopEvent::GamePauseStarted {
            timestamp: 10.0,
            pause_type_mask: 1 << 2,
        })
    );
    assert_eq!(
        tracker.apply_transition(11.0, (1 << 2) | (1 << 6)),
        Some(TimeStopEvent::GamePauseMaskChanged {
            timestamp: 11.0,
            pause_type_mask: (1 << 2) | (1 << 6),
        })
    );
    assert_eq!(
        tracker.apply_transition(12.0, 1 << 6),
        Some(TimeStopEvent::GamePauseMaskChanged {
            timestamp: 12.0,
            pause_type_mask: 1 << 6,
        })
    );
    assert_eq!(
        tracker.apply_transition(13.0, 0),
        Some(TimeStopEvent::GamePauseEnded {
            timestamp: 13.0,
            pause_type_mask: 1 << 6,
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
    raw_inventory_item_packet_at_level(catalog, id, item_id, character_net_id, 0, locked, discarded)
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
    let raw = raw_inventory_item_packet(catalog, id, item_id, Some(character_net_id), false, false);
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
fn inventory_reassembly_orders_fragments_across_packets() {
    const START: u8 = 0x09;
    const MID: u8 = 0x08;
    const END: u8 = 0x0c;
    type Step = (u16, &'static [(u16, u8, u8)]);
    type Case<'a> = (&'a str, &'a [Step], &'a [&'a [u8]]);
    let cases: &[Case<'_>] = &[
        (
            "stale fragments are replaced after sequence wrap",
            &[
                (100, &[(1023, START, 0xa1), (1, END, 0xa3)]),
                (200, &[(1023, START, 0xb1), (0, MID, 0xb2), (1, END, 0xb3)]),
            ],
            &[&[0xb1, 0xb2, 0xb3]],
        ),
        (
            "a new start does not join stale future fragments",
            &[
                (100, &[(12, MID, 0xa3), (13, END, 0xa4)]),
                (200, &[(10, START, 0xb1), (11, MID, 0xb2)]),
                (201, &[(12, MID, 0xb3), (13, END, 0xb4)]),
            ],
            &[&[0xb1, 0xb2, 0xb3, 0xb4]],
        ),
        (
            "fragment retransmitted in a newer packet",
            &[
                (100, &[(12, END, 0xa3)]),
                (200, &[(10, START, 0xa1), (11, MID, 0xa2)]),
                (201, &[(12, END, 0xa3)]),
            ],
            &[&[0xa1, 0xa2, 0xa3]],
        ),
        (
            "newer fragments arrive before their start",
            &[
                (1_867, &[(810, MID, 0xa2)]),
                (1_868, &[(811, MID, 0xa3)]),
                (1_866, &[(809, START, 0xa1)]),
                (1_869, &[(812, END, 0xa4)]),
            ],
            &[&[0xa1, 0xa2, 0xa3, 0xa4]],
        ),
        (
            "late retransmitted start fragment",
            &[
                (4_684, &[(992, MID, 0xa2)]),
                (4_685, &[(993, MID, 0xa3)]),
                (4_686, &[(994, END, 0xa4)]),
                (4_763, &[(991, START, 0xa1)]),
            ],
            &[&[0xa1, 0xa2, 0xa3, 0xa4]],
        ),
        (
            "transport packet id wrap",
            &[
                ((1 << 14) - 1, &[(100, START, 0xa1)]),
                (0, &[(101, END, 0xa2)]),
            ],
            &[&[0xa1, 0xa2]],
        ),
        (
            "stream finishes before the next start in the same packet",
            &[
                (
                    1_912,
                    &[(821, START, 0xa1), (822, MID, 0xa2), (823, MID, 0xa3)],
                ),
                (1_915, &[(824, END, 0xa4), (825, START, 0xb1)]),
                (1_916, &[(826, END, 0xb2)]),
            ],
            &[&[0xa1, 0xa2, 0xa3, 0xa4], &[0xb1, 0xb2]],
        ),
    ];
    for (name, steps, expected) in cases {
        let mut state = InventoryConnectionState::default();
        let completed = steps
            .iter()
            .flat_map(|(packet_id, bunches)| {
                let bunches = bunches
                    .iter()
                    .map(|&(sequence, flags, data)| inventory_bunch(sequence, flags, data))
                    .collect();
                state.push_bunches(*packet_id, bunches)
            })
            .map(|payload| {
                assert_eq!(payload.bit_len, payload.data.len() * 8, "{name}");
                payload.data
            })
            .collect::<Vec<_>>();
        assert_eq!(completed, *expected, "{name}");
    }
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

    let result = decoder.process_packet(connection, &character_owner_packet(1020, owner_net_id));

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

    let result = decoder.process_packet(connection, &character_owner_packet(1020, owner_net_id));

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

    let duplicate = raw_inventory_item_packet(&decoder.catalog, id, item_id, None, false, false);
    let result = decoder.process_packet(connection.clone(), &duplicate);
    assert!(result.recognized);
    assert!(result.snapshot.is_none());

    let upgrade =
        raw_inventory_item_packet_at_level(&decoder.catalog, id, item_id, None, 20, false, false);
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

    let other_connection = InventoryConnectionKey::new("other:1".to_owned(), "client:1".to_owned());
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

fn split_inventory_item_packets(
    catalog: &EquipmentCatalog,
    id: HtItemNetId,
) -> (SequencedPacket, SequencedPacket) {
    let raw = raw_inventory_item_packet(catalog, id, "Cosmos_purple", None, false, false);
    let split = raw.payload_bit_len / 2;
    let mut parts = Vec::new();
    for (start, end, sequence, flags) in [
        (0, split, 700, 0x09),
        (split, raw.payload_bit_len, 701, 0x0c),
    ] {
        let mut record = InventoryTestBitWriter::default();
        for index in start..end {
            record.push_bits(u64::from((raw.payload[index / 8] >> (index % 8)) & 1), 1);
        }
        parts.push(inventory_fragment_packet(record, sequence, flags));
    }
    let tail = parts.pop().expect("tail fragment");
    let start = parts.pop().expect("start fragment");
    (start, tail)
}

#[test]
fn inventory_mode_mismatch_does_not_seed_or_advance_fragment_clock() {
    let catalog = load_equipment_catalog(Path::new(EQUIPMENT_CATALOG_PATH))
        .expect("bundled equipment catalog should load");
    let connection = InventoryConnectionKey::new("server:1".to_owned(), "client:1".to_owned());
    let id = HtItemNetId {
        solt: 500,
        serial: 600,
    };
    let owner = HtItemNetId {
        solt: 30,
        serial: 40,
    };
    // Exercise initial clock poisoning, in-flight poisoning, and 14-bit wrap.
    // Each item spans both fragments, so raw per-packet scanning cannot mask a loss.
    for mode in [1, 2, 3] {
        for (first_id, last_id) in [(13_775, 13_776), (16_383, 0), (100, 101)] {
            let (mut start, mut tail) = split_inventory_item_packets(&catalog, id);
            start.packet_id = first_id;
            tail.packet_id = last_id;
            let mut decoder = EmptyCurtainDecoder::new(catalog.clone());
            let mut other_mode = raw_character_owner_packet(1020, owner);
            other_mode.mode = mode;
            other_mode.packet_id = 257;
            assert!(
                decoder
                    .process_packet(connection.clone(), &other_mode)
                    .snapshot
                    .is_none()
            );
            assert!(
                decoder
                    .process_packet(connection.clone(), &start)
                    .snapshot
                    .is_none()
            );
            other_mode.packet_id = 1_000;
            assert!(
                decoder
                    .process_packet(connection.clone(), &other_mode)
                    .snapshot
                    .is_none()
            );
            let result = decoder.process_packet(connection.clone(), &tail);
            let snapshot = result
                .snapshot
                .expect("both fragments must produce the item");
            assert_eq!(snapshot.len(), 1);
            assert_eq!(snapshot[0].id, id);
            assert_eq!(snapshot[0].item_id, "Cosmos_purple");
            // Only the fragment clock is gated: raw character declarations still survive.
            let characters = result
                .characters
                .expect("character instances should publish");
            assert_eq!(characters.len(), 1);
            assert_eq!(characters[0].net_id, owner);
            assert_eq!(characters[0].character_id, 1020);
        }
    }
}

#[test]
fn inventory_supported_empty_packet_still_expires_old_fragments() {
    let catalog = load_equipment_catalog(Path::new(EQUIPMENT_CATALOG_PATH))
        .expect("bundled equipment catalog should load");
    let connection = InventoryConnectionKey::new("server:1".to_owned(), "client:1".to_owned());
    let id = HtItemNetId {
        solt: 500,
        serial: 600,
    };
    let (mut start, mut tail) = split_inventory_item_packets(&catalog, id);
    start.packet_id = 100;
    let empty = SequencedPacket {
        packet_id: 197,
        payload: Vec::new(),
        payload_bit_len: 0,
        ..start.clone()
    };
    // The retransmitted tail has an in-window sequence relative to the start.
    // Only the intervening supported empty packet proves that the start has expired.
    tail.packet_id = 101;
    let mut decoder = EmptyCurtainDecoder::new(catalog);
    for packet in [&start, &empty, &tail] {
        assert!(
            decoder
                .process_packet(connection.clone(), packet)
                .snapshot
                .is_none()
        );
    }
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
            inventory_equipment_item(removed, "Nature_orange", Some(character_net_id), Some(1023)),
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
    let json =
        serde_json::to_string_pretty(&document).expect("capture export snapshot should serialize");
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
    assert_eq!(restored.packets[0].declared_ids, Vec::<u32>::new());
    assert_eq!(restored.time_stop_events, state.time_stop_events);
}

#[test]
fn capture_export_rejects_unknown_versions() {
    let error = parse_capture_export(r#"{"version":2,"hits":[],"packets":[]}"#)
        .expect_err("unknown capture export versions must be rejected");
    assert!(error.contains("unsupported capture export version 2"));
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
    assert!(parse_capture_export(packet_document(serde_json::json!("1,2"))).is_err());
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
    std::fs::write(&path, b"existing export").expect("existing export fixture should be writable");
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
    let path = directory.join("nte_raw_active.pcapng");
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
    let mut writer = RawCaptureWriter::create(&path, &device, CaptureLinkType::RawIpv4).unwrap();
    writer
        .write_packet(Duration::from_secs(1), raw_ipv4.len() as u32, raw_ipv4)
        .unwrap();
    #[cfg(windows)]
    {
        let cleanup = crate::storage::capture_logs::clear_capture_logs(&directory);
        assert_eq!(
            cleanup.deleted, 0,
            "the active capture must survive cleanup"
        );
        assert_eq!(cleanup.failed, 1);
        writer
            .write_packet(Duration::from_secs(2), raw_ipv4.len() as u32, raw_ipv4)
            .unwrap();
    }
    writer.finish().unwrap();

    let file = File::open(&path).unwrap();
    let mut reader = PcapNgReader::new(file).unwrap();
    while reader.interfaces().is_empty() {
        assert!(reader.next_block().is_some());
    }
    assert_eq!(reader.interfaces()[0].linktype, DataLink::RAW);
    drop(reader);
    let cleanup = crate::storage::capture_logs::clear_capture_logs(&directory);
    assert_eq!(cleanup.deleted, 1, "finished captures remain removable");
    assert_eq!(cleanup.failed, 0);
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
fn reaction_2_gameplay_effect_is_not_authoritative_fuwen_damage() {
    const REACTION_2_DAMAGE_EFFECT: &str = "GE_ActorReaction_2_new_Damage";
    let names = HashMap::from([(555, REACTION_2_DAMAGE_EFFECT.to_owned())]);
    let effects = [ParsedGameplayEffect {
        unique_index: 555,
        byte_offset: 0,
        bit_shift: 0,
    }];
    let mut hit = targetless_hit();

    enrich_hit_with_gameplay_effect(&mut hit, &effects, &names, &AbilityCatalog::default(), None);

    assert_eq!(hit.gameplay_effect_index, Some(555));
    assert_eq!(
        hit.gameplay_effect_name.as_deref(),
        Some(REACTION_2_DAMAGE_EFFECT)
    );
    assert_eq!(hit.attack_type.as_deref(), Some("其他"));
}

#[test]
fn hp_residual_does_not_prove_fuwen_without_display_type() {
    let mut decoder = PacketDecoder::with_server_damage_calibration(true);
    let warm_up = boss_hp_update(1_000_000.0);
    let _ = decoder.reconcile_boss_hp_updates(0.0, std::slice::from_ref(&warm_up));

    let mut hit = targetless_hit();
    hit.timestamp = 0.1;
    hit.char_id = 1;
    hit.target_max_hp = 1_000_000.0;
    hit.target_hp_before = 1_000_000.0;
    hit.target_hp_after = 999_000.0;
    hit.damage = 1_000.0;
    set_wire_target(&mut hit, [7; 29]);
    let _ = decoder.observe_server_damage_hit(&hit);

    let residual_update = boss_hp_update(998_750.0);
    let (follow_ups, _, _) =
        decoder.reconcile_boss_hp_updates(0.2, std::slice::from_ref(&residual_update));

    assert!(
        follow_ups.is_empty(),
        "only EDamageDisPlayType::DisplayType_LingZhouReactionFollow may create 覆纹"
    );
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
        declared_ids: vec![1],
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
        declared_ids: vec![],
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
        declared_ids: vec![1],
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
        declared_ids: Vec::new(),
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
fn summary_payload_matches_full_debug_for_every_bit_shift() {
    let identifier = b"Abyss_12_6_1";
    let mut length_prefixed = ((identifier.len() + 1) as u32).to_le_bytes().to_vec();
    length_prefixed.extend_from_slice(identifier);
    length_prefixed.push(0);
    let cases: [(&str, &[u8]); 3] = [
        (
            "markers",
            b"FAbyssGamePlayData ConditionState_Success EAbyssFightStage::SecondHalf AbyssCloneCharacterItemData Event.Montage.Player.UltraSkillB",
        ),
        ("explicit stage", b"  Abyss_12_6_1  "),
        ("length-prefixed stage", &length_prefixed),
    ];
    for (name, text) in cases {
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
                "{name}: marker parity failed at bit shift {bit_shift}"
            );
            assert_eq!(
                format!("{:?}", abyss_events_from_text(30.0, &summary.text)),
                format!("{:?}", abyss_events_from_text(30.0, &full.text)),
                "{name}: Abyss event parity failed at bit shift {bit_shift}"
            );
        }
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

    assert!(!dedup.is_duplicate_with_fingerprint(b"first frame", Some(10.0), forced_fingerprint,));
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
    assert!(dedup.retained_verification_bytes <= 8);
    assert_eq!(dedup.recent.len(), 2);
    assert!(
        !dedup.is_duplicate_with_fingerprint(b"123456", Some(10.000_02), 1),
        "an evicted verification body must cause a safe false negative, not hash-only dedup"
    );
    assert!(dedup.retained_verification_bytes <= 8);
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

    enrich_hit_with_gameplay_effect(&mut hit, &effects, &names, &AbilityCatalog::default(), None);

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

    enrich_hit_with_gameplay_effect(&mut hit, &effects, &names, &AbilityCatalog::default(), None);

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
fn reaction_effect_keeps_direction_without_guessing_display_type() {
    let effects = [ParsedGameplayEffect {
        unique_index: 4010,
        byte_offset: 0,
        bit_shift: 0,
    }];
    let names = HashMap::from([(4010, "GE_ActorReaction_1_Damage".to_owned())]);
    let mut hit = targetless_hit();
    hit.direction = HitDirection::Incoming;

    enrich_hit_with_gameplay_effect(&mut hit, &effects, &names, &AbilityCatalog::default(), None);

    assert_eq!(hit.direction, HitDirection::Incoming);
    assert_eq!(hit.attack_type.as_deref(), Some("其他"));
}

#[test]
fn reaction_catalog_metadata_cannot_replace_authoritative_display_type() {
    let effect = ParsedGameplayEffect {
        unique_index: 4010,
        byte_offset: 0,
        bit_shift: 0,
    };
    let names = HashMap::from([(4010, "GE_ActorReaction_1_Damage".to_owned())]);
    let catalog = AbilityCatalog::from(HashMap::from([(
        "GE_ActorReaction_1_Damage".to_owned(),
        GameplayEffectSkill {
            damage_source_category: Some("R".to_owned()),
            ability_name: None,
            attack_type: "创生花".to_owned(),
            damage_component: Some("Blossom Damage".to_owned()),
            owner_character_id: None,
            use_server_damage: false,
            max_hp_reduction_percent: 0,
        },
    )]));
    let mut hit = targetless_hit();
    hit.attack_type = Some("环合·创生".to_owned());

    apply_gameplay_effect(&mut hit, &effect, &names, &catalog);

    assert_eq!(hit.gameplay_effect_index, Some(4010));
    assert_eq!(hit.attack_type, None);
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
    assert_eq!(creation_flower.attack_type.as_deref(), Some("其他"));
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

    let primary_bit_offset = hit_bit_offset + BOOL_ENUM_DAMAGE_RECORD_TO_GAMEPLAY_EFFECT_BITS[0];
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

    let alternate_bit_offset = hit_bit_offset + BOOL_ENUM_DAMAGE_RECORD_TO_GAMEPLAY_EFFECT_BITS[1];
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
fn reaction_buff_keeps_direction_without_guessing_display_type() {
    let effects = [ParsedGameplayEffect {
        unique_index: 4011,
        byte_offset: 0,
        bit_shift: 0,
    }];
    let names = HashMap::from([(4011, "Buff_Reaction_4_new".to_owned())]);
    let mut hit = targetless_hit();
    hit.direction = HitDirection::Incoming;

    enrich_hit_with_gameplay_effect(&mut hit, &effects, &names, &AbilityCatalog::default(), None);

    assert_eq!(hit.direction, HitDirection::Incoming);
    assert_eq!(hit.attack_type.as_deref(), Some("其他"));
}

#[test]
fn tenacity_effect_keeps_direction_without_guessing_unbalance_type() {
    let effects = [ParsedGameplayEffect {
        unique_index: 4012,
        byte_offset: 0,
        bit_shift: 0,
    }];
    let names = HashMap::from([(4012, "Buff_Tenacity_damage".to_owned())]);
    let mut hit = targetless_hit();
    hit.direction = HitDirection::Incoming;

    enrich_hit_with_gameplay_effect(&mut hit, &effects, &names, &AbilityCatalog::default(), None);

    assert_eq!(hit.direction, HitDirection::Incoming);
    assert_eq!(hit.attack_type.as_deref(), Some("其他"));
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

    enrich_hit_with_gameplay_effect(&mut hit, &effects, &names, &AbilityCatalog::default(), None);

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

    enrich_hit_with_gameplay_effect(&mut hit, &effects, &names, &AbilityCatalog::default(), None);

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

    enrich_hit_with_gameplay_effect(&mut hit, &effects, &names, &AbilityCatalog::default(), None);

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

    enrich_hit_with_gameplay_effect(&mut hit, &effects, &names, &AbilityCatalog::default(), None);

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

    enrich_hit_with_gameplay_effect(&mut hit, &effects, &names, &AbilityCatalog::default(), None);

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

#[test]
fn exported_native_hit_keeps_snapshot_through_json_replay() {
    use crate::engine::model::plugin_snapshot::PluginHitSnapshot;
    let mut hit = targetless_hit();
    hit.plugin_snapshot = Some(
        PluginHitSnapshot {
            instance_id: 0,
            key: "capture:7".into(),
            critical: Some(false),
            critical_source: Some("native_prediction".into()),
            role_effects: None,
            enemy_effects: None,
            retention: "missing".into(),
            data: None,
        }
        .seal()
        .unwrap(),
    );
    let exported = ExportHit::from(&hit);
    let exported: ExportHit =
        serde_json::from_slice(&serde_json::to_vec(&exported).unwrap()).unwrap();
    let EngineEvent::Hit(restored) = export_hit_event(exported) else {
        panic!()
    };
    assert_eq!(restored.plugin_snapshot.as_ref().unwrap().key, "capture:7");
    assert_eq!(
        restored.plugin_snapshot.as_ref().unwrap().critical,
        Some(false)
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
        exact: None,
        plugin_snapshot: None,
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
        source_character_id: None,
        current_hp,
        dead_state,
        raw_damage,
        display_type: DamageDisplayType::None,
        additional_damage: None,
        additional_display_type: None,
        byte_offset: 0,
        bit_shift: 0,
    }
}

#[test]
fn reassembled_settlement_completes_source_without_double_counting() {
    let mut unknown = server_damage_settlement_for([1; 29], 900.0, 0, 100);
    unknown.additional_damage = Some(33);
    unknown.additional_display_type = Some(DamageDisplayType::LingZhouReactionFollow);
    let mut known = unknown.clone();
    known.source_character_id = Some(1036);
    known.byte_offset = 100; // Reassembly shifts the containing buffer.
    for (direct, reassembled) in [
        (unknown.clone(), known.clone()),
        (known.clone(), unknown.clone()),
    ] {
        let offset = direct.byte_offset;
        let mut rows = vec![direct];
        merge_reassembled_server_damage_settlements(&mut rows, vec![reassembled]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].source_character_id, Some(1036));
        assert_eq!(rows[0].additional_damage, Some(33));
        assert_eq!(rows[0].byte_offset, offset);
    }
    let mut rows = vec![unknown];
    merge_reassembled_server_damage_settlements(&mut rows, vec![known.clone(), known]);
    assert_eq!(rows.len(), 2); // Two real occurrences must not collapse.
    assert!(rows.iter().all(|row| row.source_character_id == Some(1036)));
    assert_eq!(
        rows.iter()
            .map(|row| row.additional_damage.unwrap_or(0))
            .sum::<u32>(),
        66
    );
}

#[test]
fn reassembled_settlement_reserves_exact_roles_before_missing_sources() {
    let unknown = server_damage_settlement_for([1; 29], 900.0, 0, 100);
    let mut first = unknown.clone();
    first.source_character_id = Some(1036);
    let mut second = unknown.clone();
    second.source_character_id = Some(1075);
    for (direct, reassembled) in [
        (
            vec![first.clone(), second.clone()],
            vec![unknown.clone(), first.clone()],
        ),
        (
            vec![unknown.clone(), first.clone()],
            vec![second.clone(), first.clone()],
        ),
        (
            vec![unknown.clone(), first.clone()],
            vec![unknown.clone(), second.clone()],
        ),
        (
            vec![unknown.clone(), first.clone()],
            vec![second.clone(), unknown.clone()],
        ),
    ] {
        let mut rows = direct;
        merge_reassembled_server_damage_settlements(&mut rows, reassembled);
        assert_eq!(rows.len(), 2);
        let mut roles = rows
            .iter()
            .filter_map(|row| row.source_character_id)
            .collect::<Vec<_>>();
        roles.sort_unstable();
        assert_eq!(roles, vec![1036, 1075]);
    }
    let mut rows = vec![first];
    merge_reassembled_server_damage_settlements(&mut rows, vec![second]);
    assert_eq!(rows.len(), 2); // Conflicting known declarations remain distinct.
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
    let settlement = server_damage_settlement_for(target, 1_900.0, 0, 100);

    let resolved =
        decoder.resolve_pending_hit_targets(10.1, &[update], std::slice::from_ref(&settlement));

    assert_eq!(resolved.len(), 1);
    assert_eq!(wire_handle_from_hit(&resolved[0]), Some(target));
    assert!(decoder.pending_targetless_hits.is_empty());
}

#[test]
fn hp_resolved_hit_waits_for_and_yields_to_matching_direct_target_record() {
    let mut decoder = PacketDecoder::default();
    let target = [4_u8; 29];
    let mut provisional = targetless_hit();
    provisional.timestamp = 10.0;
    provisional.char_id = 1051;
    provisional.char_source = HitCharacterSource::Packet;
    provisional.damage = 1_350.0;
    provisional.target_hp_before = 5_276_334.0;
    provisional.target_hp_after = 5_274_984.0;
    provisional.target_max_hp = 5_417_573.0;
    provisional.gameplay_effect_index = Some(2417);
    decoder.pending_targetless_hits.push_back(provisional);
    let update = boss_hp_update_for(target, 5_274_984.0);
    decoder.observe_target_hp_update(10.01, &update);

    assert!(
        decoder
            .resolve_pending_hit_targets(10.01, &[update], &[])
            .is_empty()
    );
    assert_eq!(decoder.pending_targetless_hits.len(), 1);
    assert_eq!(
        wire_handle_from_hit(&decoder.pending_targetless_hits[0]),
        Some(target)
    );

    let mut direct = decoder.pending_targetless_hits[0].clone();
    direct.timestamp = 10.066_759;
    set_wire_target(&mut direct, target);
    let prepared = decoder.prepare_hits_for_emission(vec![direct], &[1051], false, &HashMap::new());

    assert_eq!(prepared.emit.len(), 1);
    assert_eq!(prepared.suppressed_ambiguous, 1);
    assert!(decoder.pending_targetless_hits.is_empty());
    assert!(has_direct_wire_target(&prepared.emit[0]));
}

#[test]
fn direct_record_for_another_target_does_not_suppress_hp_resolved_hit() {
    let mut decoder = PacketDecoder::default();
    let resolved_target = [4_u8; 29];
    let mut provisional = targetless_hit();
    provisional.timestamp = 10.0;
    provisional.char_id = 1051;
    provisional.char_source = HitCharacterSource::Packet;
    provisional.target_hp_before = 2_000.0;
    provisional.target_hp_after = 1_900.0;
    provisional.target_max_hp = 2_500.0;
    provisional.gameplay_effect_index = Some(2417);
    decoder.pending_targetless_hits.push_back(provisional);
    let update = boss_hp_update_for(resolved_target, 1_900.0);
    decoder.observe_target_hp_update(10.01, &update);
    assert!(
        decoder
            .resolve_pending_hit_targets(10.01, &[update], &[])
            .is_empty()
    );

    let mut other_target = decoder.pending_targetless_hits[0].clone();
    other_target.timestamp = 10.066_759;
    set_wire_target(&mut other_target, [5_u8; 29]);
    let prepared =
        decoder.prepare_hits_for_emission(vec![other_target], &[1051], false, &HashMap::new());

    assert_eq!(prepared.emit.len(), 1);
    assert_eq!(prepared.suppressed_ambiguous, 0);
    assert_eq!(decoder.pending_targetless_hits.len(), 1);
    assert_eq!(
        wire_handle_from_hit(&decoder.pending_targetless_hits[0]),
        Some(resolved_target)
    );
}

#[test]
fn unrelated_server_settlement_does_not_release_hp_resolved_hit() {
    let mut decoder = PacketDecoder::default();
    let resolved_target = [4_u8; 29];
    let mut provisional = targetless_hit();
    provisional.timestamp = 10.0;
    provisional.target_hp_before = 2_000.0;
    provisional.target_hp_after = 1_900.0;
    provisional.target_max_hp = 2_500.0;
    decoder.pending_targetless_hits.push_back(provisional);
    let update = boss_hp_update_for(resolved_target, 1_900.0);
    decoder.observe_target_hp_update(10.01, &update);
    let unrelated = server_damage_settlement_for([5_u8; 29], 900.0, 0, 100);

    let resolved = decoder.resolve_pending_hit_targets(10.01, &[update], &[unrelated]);

    assert!(resolved.is_empty());
    assert_eq!(decoder.pending_targetless_hits.len(), 1);
    assert_eq!(
        wire_handle_from_hit(&decoder.pending_targetless_hits[0]),
        Some(resolved_target)
    );
}

#[test]
fn exact_server_target_response_promotes_pending_unknown_direction() {
    let mut decoder = PacketDecoder::default();
    let target = [4_u8; 29];
    let mut hit = targetless_hit();
    hit.timestamp = 10.0;
    hit.direction = HitDirection::Unknown;
    hit.char_source = HitCharacterSource::Session;
    hit.target_hp_before = 2_000.0;
    hit.target_hp_after = 1_900.0;
    hit.target_max_hp = 2_500.0;
    decoder.pending_ambiguous_hits.push(hit);
    let update = boss_hp_update_for(target, 1_900.0);
    decoder.observe_target_hp_update(10.1, &update);

    assert!(
        decoder
            .resolve_pending_hit_targets(10.1, &[update], &[])
            .is_empty()
    );

    assert_eq!(decoder.pending_ambiguous_hits.len(), 1);
    assert_eq!(
        wire_handle_from_hit(&decoder.pending_ambiguous_hits[0]),
        Some(target)
    );
    assert_eq!(
        decoder.pending_ambiguous_hits[0].direction,
        HitDirection::Outgoing
    );
}

#[test]
fn ambiguous_targetless_hit_keeps_client_hit_and_server_residual() {
    let mut decoder = PacketDecoder::default();
    let mut hit = targetless_hit();
    hit.timestamp = 10.0;
    hit.damage = 100.0;
    hit.target_hp_before = 2_000.0;
    hit.target_hp_after = 1_900.0;
    hit.target_max_hp = 2_500.0;
    decoder.pending_targetless_hits.push_back(hit);

    let first = boss_hp_update_for([4_u8; 29], 1_900.0);
    let second = boss_hp_update_for([5_u8; 29], 1_900.0);
    decoder.observe_target_hp_update(10.1, &first);
    decoder.observe_target_hp_update(10.1, &second);
    assert!(
        decoder
            .resolve_pending_hit_targets(10.1, &[first, second], &[])
            .is_empty()
    );

    let settlement = server_damage_settlement_for([4_u8; 29], 1_900.0, 0, 100);
    let (corrections, residuals, _) =
        decoder.reconcile_server_damage_settlements(10.1, &[settlement]);
    let client_hits = decoder.take_expired_targetless_hits(10.51);

    assert!(corrections.is_empty());
    assert_eq!(
        residuals,
        vec![UnattributedServerDamage {
            timestamp: 10.1,
            damage: 100.0,
            candidate_hits: 0,
        }]
    );
    assert_eq!(client_hits.len(), 1);
    assert_eq!(client_hits[0].damage, 100.0);
    assert!(client_hits[0].target_id.is_none());
}

#[test]
fn targetless_hit_then_terminal_settlement_is_counted_once() {
    let mut decoder = PacketDecoder::default();
    let target = [4_u8; 29];
    let mut hit = targetless_hit();
    hit.timestamp = 10.0;
    hit.target_hp_before = 99.0;
    hit.target_hp_after = 0.0;
    hit.target_max_hp = 100.0;
    hit.damage = 99.0;
    decoder.pending_targetless_hits.push_back(hit);
    let settlement = server_damage_settlement_for(target, 1.0, 1, 99);
    let update = crate::engine::parser::ParsedBossHpUpdate {
        target_handle: settlement.target_handle,
        current_hp: settlement.current_hp,
        byte_offset: settlement.byte_offset,
        bit_shift: settlement.bit_shift,
    };
    decoder.observe_target_hp_update(10.1, &update);

    let resolved =
        decoder.resolve_pending_hit_targets(10.1, &[update], std::slice::from_ref(&settlement));
    let (_, _, residual_hits) = decoder.reconcile_current_packet_server_damage_settlements(
        10.1,
        &[settlement],
        resolved.iter(),
    );

    assert_eq!(resolved.len(), 1);
    assert_eq!(wire_handle_from_hit(&resolved[0]), Some(target));
    assert!(decoder.pending_targetless_hits.is_empty());
    assert_eq!(residual_hits.len(), 1);
    assert_eq!(residual_hits[0].damage, 0.0);
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
            .resolve_pending_hit_targets(10.1, &[first, second], &[])
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
            .resolve_pending_hit_targets(10.1, &[update], &[])
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
fn emission_drops_outgoing_death_settlement_marker() {
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

    marker.target_id = None;
    marker.target_context.clear();
    decoder.emit_hits(std::iter::once(marker.clone()), &characters, &sink);
    assert!(receiver.try_recv().is_err());

    marker.direction = HitDirection::Incoming;
    decoder.emit_hits(std::iter::once(marker), &characters, &sink);
    assert!(matches!(receiver.try_recv(), Ok(EngineEvent::Hit(_))));
}

#[test]
fn exact_server_damage_settlement_reconciles_client_overkill() {
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
    assert_eq!(correction.reconciled_overkill_damage, Some(0.0));

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
        Some(0.0)
    );
}

#[test]
fn server_authoritative_special_damage_waits_for_max_hp_evidence() {
    let target = [8_u8; 29];
    let mut tracker = ServerDamageCalibrationTracker::default();
    let mut nightmare = duplicate_test_hit(10.0, HitCharacterSource::Packet, "outgoing");
    nightmare.damage = 800.0;
    nightmare.target_hp_before = 8_000.0;
    nightmare.target_hp_after = 7_200.0;
    nightmare.target_max_hp = 10_000.0;
    set_wire_target(&mut nightmare, target);
    let _ = tracker.observe_hit_with_semantics(&nightmare, true, 200);

    let (correction, unattributed) = tracker.observe_server_damage_settlement(
        10.05,
        &server_damage_settlement_for(target, 7_000.0, 0, 1_000),
    );

    let correction = correction.expect("server-authoritative damage should be corrected");
    assert!(unattributed.is_none());
    assert_eq!(correction.damage, 1_000.0);
    assert_eq!(correction.reconciled_overkill_damage, Some(0.0));
    assert_eq!(correction.max_hp_reduction, None);
    assert_eq!(tracker.pending_max_hp_reduction_hits.len(), 1);
}

#[test]
fn normal_nightmare_settlement_does_not_reduce_max_hp_without_a_scaled_hp_transition() {
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

    let (correction, unattributed) = tracker.observe_server_damage_settlement(
        10.05,
        &server_damage_settlement_for(target, 7_000.0, 0, 1_000),
    );

    assert!(unattributed.is_none());
    assert_eq!(correction.unwrap().max_hp_reduction, None);
    assert_eq!(tracker.target_max_hp_by_handle[&target], 10_000.0);
}

#[test]
fn scaled_nightmare_hp_transition_confirms_max_hp_reduction_immediately() {
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

    // Server raw damage is 1,000. After that direct damage, the game keeps
    // the HP ratio while reducing max HP by 2,000:
    // (8,000 - 1,000) * (10,000 - 2,000) / 10,000 = 5,600.
    let (correction, unattributed) = tracker.observe_server_damage_settlement(
        10.05,
        &server_damage_settlement_for(target, 5_600.0, 0, 1_000),
    );

    assert!(unattributed.is_none());
    assert_eq!(correction.unwrap().max_hp_reduction, Some(2_000.0));
    assert_eq!(tracker.target_max_hp_by_handle[&target], 8_000.0);
    assert!(tracker.pending_max_hp_reduction_hits.is_empty());
}

#[test]
fn max_hp_candidate_remains_valid_until_target_death() {
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

    let mut later_hit = duplicate_test_hit(600.0, HitCharacterSource::Packet, "outgoing");
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
        let mut nightmare = duplicate_test_hit(timestamp, HitCharacterSource::Packet, "outgoing");
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
    let mut decoder = PacketDecoder::with_ability_catalog(catalog.into(), true);
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

    let (corrections, unattributed, residual_hits) = decoder.reconcile_server_damage_settlements(
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
fn primary_display_type_replaces_heuristic_metadata_with_exact_enum_label() {
    let target = [7_u8; 29];
    let mut tracker = ServerDamageCalibrationTracker::default();
    let mut hit = duplicate_test_hit(10.0, HitCharacterSource::Packet, "outgoing");
    hit.damage = 800.0;
    hit.target_hp_before = 10_000.0;
    hit.target_hp_after = 9_200.0;
    hit.target_max_hp = 10_000.0;
    hit.attack_type = Some("其他".to_owned());
    set_wire_target(&mut hit, target);
    tracker.observe_hit(&hit);
    let mut settlement = server_damage_settlement_for(target, 9_079.0, 0, 921);
    settlement.display_type = DamageDisplayType::Unbal;

    let (correction, unattributed) = tracker.observe_server_damage_settlement(10.05, &settlement);
    let correction = correction.expect("the exact target settlement should match its hit");

    assert!(unattributed.is_none());
    assert_eq!(correction.damage, 921.0);
    assert_eq!(correction.damage_name.as_deref(), Some("倾陷伤害"));
    assert_eq!(correction.attack_type.as_deref(), Some("倾陷伤害"));
}

#[test]
fn unmatched_primary_reaction_display_type_emits_exact_unattributed_damage() {
    let target = [7_u8; 29];
    let mut tracker = ServerDamageCalibrationTracker::default();
    tracker.target_max_hp_by_handle.insert(target, 10_000.0);
    let mut settlement = server_damage_settlement_for(target, 9_503.0, 0, 497);
    settlement.display_type = DamageDisplayType::LingZhouReactionFollow;

    let (corrections, unattributed) =
        tracker.observe_server_damage_settlements(10.05, &[settlement]);
    let residual_hits = tracker.take_residual_hits();

    assert!(corrections.is_empty());
    assert!(unattributed.is_empty());
    assert_eq!(residual_hits.len(), 1);
    assert_eq!(residual_hits[0].damage, 497.0);
    assert_eq!(
        residual_hits[0].damage_name.as_deref(),
        Some("覆纹追加攻击")
    );
    assert_eq!(residual_hits[0].attack_type.as_deref(), Some("覆纹"));
    assert!(!residual_hits[0].char_known);
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

    let _ = tracker
        .observe_server_damage_settlement(10.06, &server_damage_settlement_for(target, 1.0, 1, 2));
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

    let (_, observation) = tracker.observe_boss_hp_detailed(2.0, &boss_hp_update_for(first, 900.0));

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
fn authoritative_four_wrapper_display_type_uses_unique_wire_source_and_exact_damage() {
    let mut decoder = PacketDecoder::default();
    let target = [7_u8; 29];
    let mut source = targetless_hit();
    source.timestamp = 10.0;
    source.char_id = 1;
    source.damage = 1_000.0;
    source.target_hp_before = 10_000.0;
    source.target_hp_after = 9_000.0;
    source.target_max_hp = 10_000.0;
    set_wire_target(&mut source, target);
    let mut settlement = server_damage_settlement_for(target, 8_750.0, 0, 1_000);
    settlement.additional_damage = Some(250);
    settlement.additional_display_type = Some(DamageDisplayType::LingZhouReactionFollow);

    let settlements = [settlement];
    let reconciliation = decoder.reconcile_current_packet_server_damage_settlements_with_sources(
        10.05,
        &settlements,
        [&source],
    );
    let (follow_ups, shared_hits) = decoder.reconcile_authoritative_additional_damage_settlements(
        10.05,
        &settlements,
        &reconciliation.sources,
    );

    assert!(shared_hits.is_empty());
    assert_eq!(reconciliation.corrections.len(), 1);
    assert_eq!(follow_ups.len(), 1);
    assert_eq!(follow_ups[0].damage, 250.0);
    assert_eq!(follow_ups[0].source_char_id, 1);
}

#[test]
fn authoritative_four_wrapper_display_type_stays_unattributed_without_unique_source() {
    let mut decoder = PacketDecoder::default();
    let target = [7_u8; 29];
    let mut first = targetless_hit();
    first.timestamp = 9.2;
    first.char_id = 1;
    first.damage = 700.0;
    first.target_hp_before = 10_000.0;
    first.target_hp_after = 9_300.0;
    first.target_max_hp = 10_000.0;
    set_wire_target(&mut first, target);
    let mut second = first.clone();
    second.timestamp = 9.3;
    second.char_id = 2;
    second.damage = 500.0;
    second.target_hp_before = 9_300.0;
    second.target_hp_after = 8_800.0;
    let mut settlement = server_damage_settlement_for(target, 8_750.0, 0, 1_000);
    settlement.additional_damage = Some(250);
    settlement.additional_display_type = Some(DamageDisplayType::LingZhouReactionFollow);

    let settlements = [settlement];
    let reconciliation = decoder.reconcile_current_packet_server_damage_settlements_with_sources(
        10.0,
        &settlements,
        [&first, &second],
    );
    let (follow_ups, shared_hits) = decoder.reconcile_authoritative_additional_damage_settlements(
        10.0,
        &settlements,
        &reconciliation.sources,
    );

    assert_eq!(reconciliation.corrections.len(), 1);
    assert!(reconciliation.sources[0].is_none());
    assert!(follow_ups.is_empty());
    assert_eq!(shared_hits.len(), 1);
    assert_eq!(shared_hits[0].damage, 250.0);
    assert_eq!(shared_hits[0].char_id, 0);
    assert!(!shared_hits[0].char_known);
    assert_eq!(shared_hits[0].attack_type.as_deref(), Some("覆纹"));
}

#[test]
fn authoritative_four_wrapper_display_type_keeps_unique_recent_source_fallback() {
    let mut decoder = PacketDecoder::default();
    let target = [7_u8; 29];
    let mut source = targetless_hit();
    source.timestamp = 10.0;
    source.char_id = 1;
    source.damage = 700.0;
    source.target_hp_before = 10_000.0;
    source.target_hp_after = 9_300.0;
    source.target_max_hp = 10_000.0;
    set_wire_target(&mut source, target);
    let mut settlement = server_damage_settlement_for(target, 8_750.0, 0, 1_000);
    settlement.additional_damage = Some(250);
    settlement.additional_display_type = Some(DamageDisplayType::LingZhouReactionFollow);

    let settlements = [settlement];
    let reconciliation = decoder.reconcile_current_packet_server_damage_settlements_with_sources(
        10.05,
        &settlements,
        [&source],
    );
    let (follow_ups, shared_hits) = decoder.reconcile_authoritative_additional_damage_settlements(
        10.05,
        &settlements,
        &reconciliation.sources,
    );

    assert!(shared_hits.is_empty());
    assert_eq!(follow_ups.len(), 1);
    assert_eq!(follow_ups[0].source_timestamp, source.timestamp);
    assert_eq!(follow_ups[0].source_char_id, source.char_id);
}

#[test]
fn authoritative_four_wrapper_display_type_rejects_ambiguous_hp_bridge() {
    let mut decoder = PacketDecoder::default();
    let target = [7_u8; 29];
    let mut first = targetless_hit();
    first.timestamp = 9.2;
    first.char_id = 1;
    first.damage = 700.0;
    first.target_hp_before = 10_000.0;
    first.target_hp_after = 9_000.0;
    first.target_max_hp = 10_000.0;
    set_wire_target(&mut first, target);
    let mut second = first.clone();
    second.timestamp = 9.3;
    second.char_id = 2;
    second.damage = 500.0;
    let mut settlement = server_damage_settlement_for(target, 8_750.0, 0, 1_000);
    settlement.additional_damage = Some(250);
    settlement.additional_display_type = Some(DamageDisplayType::LingZhouReactionFollow);

    let settlements = [settlement];
    let reconciliation = decoder.reconcile_current_packet_server_damage_settlements_with_sources(
        10.0,
        &settlements,
        [&first, &second],
    );
    let (follow_ups, shared_hits) = decoder.reconcile_authoritative_additional_damage_settlements(
        10.0,
        &settlements,
        &reconciliation.sources,
    );

    assert_eq!(reconciliation.corrections.len(), 1);
    assert!(reconciliation.sources[0].is_none());
    assert!(follow_ups.is_empty());
    assert_eq!(shared_hits.len(), 1);
    assert!(!shared_hits[0].char_known);
}

#[test]
fn authoritative_settlement_claims_its_duplicate_legacy_hp_update() {
    let target = [7_u8; 29];
    let settlement = server_damage_settlement_for(target, 8_750.0, 0, 1_000);
    let duplicate = boss_hp_update_for(target, 8_750.0);
    let different = boss_hp_update_for(target, 8_749.0);

    assert!(PacketDecoder::hp_update_has_authoritative_settlement(
        std::slice::from_ref(&settlement),
        &duplicate,
    ));
    assert!(!PacketDecoder::hp_update_has_authoritative_settlement(
        &[settlement],
        &different,
    ));
}

#[test]
fn declared_weave_role_constrains_existing_source_matching() {
    // Even an exact damage/HP match from a different role cannot steal
    // the declared role's exact, HP-bridge or unique recent candidate.
    for matching_rule in ["exact", "hp_bridge", "recent"] {
        let mut decoder = PacketDecoder::default();
        let target = [7_u8; 29];
        let mut other = targetless_hit();
        other.timestamp = 10.0;
        other.char_id = 1004;
        other.damage = 1_000.0;
        other.target_hp_before = 10_000.0;
        other.target_hp_after = 9_000.0;
        other.target_max_hp = 10_000.0;
        other.byte_offset = 100;
        set_wire_target(&mut other, target);
        let mut source = other.clone();
        source.char_id = 1036;
        source.byte_offset = 200;
        if matching_rule != "exact" {
            source.damage = 700.0;
        }
        if matching_rule == "recent" {
            source.target_hp_after = 9_300.0;
        }
        let mut settlement = server_damage_settlement_for(target, 8_750.0, 0, 1_000);
        settlement.source_character_id = Some(source.char_id);
        settlement.additional_damage = Some(250);
        settlement.additional_display_type = Some(DamageDisplayType::LingZhouReactionFollow);
        let settlements = [settlement];
        let reconciliation = decoder
            .reconcile_current_packet_server_damage_settlements_with_sources(
                10.05,
                &settlements,
                [&other, &source],
            );
        let (follow_ups, shared_hits) = decoder
            .reconcile_authoritative_additional_damage_settlements(
                10.05,
                &settlements,
                &reconciliation.sources,
            );

        assert_eq!(reconciliation.corrections.len(), 1, "{matching_rule}");
        assert_eq!(reconciliation.corrections[0].source_char_id, source.char_id);
        assert_eq!(follow_ups.len(), 1, "{matching_rule}");
        assert_eq!(follow_ups[0].source_char_id, source.char_id);
        assert_eq!(follow_ups[0].source_byte_offset, Some(source.byte_offset));
        assert_eq!(follow_ups[0].source_target_id, source.target_id);
        assert_eq!(follow_ups[0].damage, 250.0);
        assert!(shared_hits.is_empty(), "{matching_rule}");
    }
}

#[test]
fn declared_weave_role_does_not_link_to_another_role_when_source_hit_is_missing() {
    let mut decoder = PacketDecoder::default();
    let target = [7_u8; 29];
    let mut other = targetless_hit();
    other.timestamp = 10.0;
    other.char_id = 1004;
    other.damage = 1_000.0;
    other.target_hp_before = 10_000.0;
    other.target_hp_after = 9_000.0;
    other.target_max_hp = 10_000.0;
    set_wire_target(&mut other, target);
    let mut settlement = server_damage_settlement_for(target, 8_750.0, 0, 1_000);
    settlement.source_character_id = Some(1036);
    settlement.additional_damage = Some(250);
    settlement.additional_display_type = Some(DamageDisplayType::LingZhouReactionFollow);
    let settlements = [settlement];
    let reconciliation = decoder.reconcile_current_packet_server_damage_settlements_with_sources(
        10.05,
        &settlements,
        [&other],
    );
    assert!(reconciliation.corrections.is_empty());
    assert!(reconciliation.sources[0].is_none());
    let (follow_ups, shared_hits) = decoder.reconcile_authoritative_additional_damage_settlements(
        10.05,
        &settlements,
        &reconciliation.sources,
    );
    assert!(follow_ups.is_empty());
    // Exhausted source matching retains the existing unknown record;
    // a role declaration alone does not create a known independent hit.
    assert_eq!(shared_hits.len(), 1);
    assert_eq!(shared_hits[0].char_id, 0);
    assert!(!shared_hits[0].char_known);
    assert_eq!(shared_hits[0].damage, 250.0);
}

#[test]
fn authoritative_four_wrapper_display_type_pairs_identical_burst_hits_in_wire_order() {
    let mut decoder = PacketDecoder::default();
    let target = [7_u8; 29];
    let mut first = targetless_hit();
    first.timestamp = 10.0;
    first.char_id = 1;
    first.damage = 2_486.0;
    first.target_hp_before = 11_140_138.0;
    first.target_hp_after = 11_137_652.0;
    first.target_max_hp = 11_245_012.0;
    first.byte_offset = 100;
    set_wire_target(&mut first, target);
    let mut second = first.clone();
    second.byte_offset = 200;

    let mut first_settlement = server_damage_settlement_for(target, 11_137_155.0, 0, 2_486);
    first_settlement.additional_damage = Some(497);
    first_settlement.additional_display_type = Some(DamageDisplayType::LingZhouReactionFollow);
    let first_settlements = [first_settlement];
    let first_reconciliation = decoder
        .reconcile_current_packet_server_damage_settlements_with_sources(
            10.057,
            &first_settlements,
            [&first, &second],
        );
    let (first_follow_ups, first_shared) = decoder
        .reconcile_authoritative_additional_damage_settlements(
            10.057,
            &first_settlements,
            &first_reconciliation.sources,
        );
    let mut second_settlement = server_damage_settlement_for(target, 11_134_172.0, 0, 2_486);
    second_settlement.additional_damage = Some(497);
    second_settlement.additional_display_type = Some(DamageDisplayType::LingZhouReactionFollow);
    let second_settlements = [second_settlement];
    let second_reconciliation = decoder
        .reconcile_current_packet_server_damage_settlements_with_sources(
            10.106,
            &second_settlements,
            std::iter::empty(),
        );
    let (second_follow_ups, second_shared) = decoder
        .reconcile_authoritative_additional_damage_settlements(
            10.106,
            &second_settlements,
            &second_reconciliation.sources,
        );

    assert!(first_shared.is_empty());
    assert!(second_shared.is_empty());
    assert_eq!(first_follow_ups[0].source_timestamp, first.timestamp);
    assert_eq!(second_follow_ups[0].source_timestamp, second.timestamp);
    assert_eq!(
        first_follow_ups[0].source_byte_offset,
        Some(first.byte_offset)
    );
    assert_eq!(
        second_follow_ups[0].source_byte_offset,
        Some(second.byte_offset)
    );
    let mut state = CombatState::default();
    state.push_hit(first);
    state.push_hit(second);
    for (follow_ups, corrections) in [
        (first_follow_ups, first_reconciliation.corrections),
        (second_follow_ups, second_reconciliation.corrections),
    ] {
        for follow_up in follow_ups {
            assert!(state.apply_follow_up(follow_up));
        }
        for correction in corrections {
            assert!(state.apply_damage_correction(correction));
        }
    }
    assert_eq!(state.hits.len(), 2);
    assert_eq!(state.hits[0].follow_up_damage, 497.0);
    assert_eq!(state.hits[1].follow_up_damage, 497.0);
}

#[test]
fn reconcile_boss_hp_updates_still_calibrates_when_no_follow_up_applies() {
    let mut decoder = PacketDecoder::with_server_damage_calibration(true);

    let warm_up = boss_hp_update(10_000.0);
    let _ = decoder.reconcile_boss_hp_updates(9.0, std::slice::from_ref(&warm_up));

    let mut hit = duplicate_test_hit(10.0, HitCharacterSource::Packet, "outgoing");
    hit.damage = 1_000.0;
    hit.target_hp_before = 10_000.0;
    hit.target_hp_after = 9_000.0;
    hit.target_max_hp = 10_000.0;
    hit.gameplay_effect_name = Some("GE_Unmapped_Damage".to_owned());
    set_wire_target(&mut hit, [7; 29]);
    let _ = decoder.observe_server_damage_hit(&hit);

    let update = boss_hp_update(8_750.0);
    let (inferred_follow_ups, server_damage_corrections, unattributed) =
        decoder.reconcile_boss_hp_updates(10.05, std::slice::from_ref(&update));

    assert!(inferred_follow_ups.is_empty());
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
    let (inferred_follow_ups, corrections, unattributed) =
        decoder.reconcile_boss_hp_updates(10.0, std::slice::from_ref(&update));

    assert!(inferred_follow_ups.is_empty());
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
fn calibration_setting_is_fallback_only_for_effects_missing_from_semantics() {
    let catalog = AbilityCatalog::from(HashMap::from([(
        "GE_Known_Client_Damage".to_owned(),
        GameplayEffectSkill {
            damage_source_category: None,
            ability_name: None,
            attack_type: "Skill Damage".to_owned(),
            damage_component: None,
            owner_character_id: None,
            use_server_damage: false,
            max_hp_reduction_percent: 0,
        },
    )]));
    let mut decoder = PacketDecoder::with_ability_catalog(catalog.into(), true);

    let mut known = duplicate_test_hit(10.0, HitCharacterSource::Packet, "outgoing");
    known.gameplay_effect_name = Some("GE_Known_Client_Damage".to_owned());
    set_wire_target(&mut known, [7; 29]);
    let _ = decoder.observe_server_damage_hit(&known);

    let mut unknown = duplicate_test_hit(10.01, HitCharacterSource::Packet, "outgoing");
    unknown.gameplay_effect_name = Some("GE_Unknown_Damage".to_owned());
    set_wire_target(&mut unknown, [8; 29]);
    let _ = decoder.observe_server_damage_hit(&unknown);

    assert!(!decoder.server_damage_calibration.pending_hits[0].use_server_damage);
    assert!(decoder.server_damage_calibration.pending_hits[1].use_server_damage);
}

#[test]
fn exact_server_settlement_overrides_client_damage_even_when_legacy_calibration_is_disabled() {
    let target = [7_u8; 29];
    let mut decoder = PacketDecoder::with_server_damage_calibration(false);
    let mut hit = duplicate_test_hit(10.0, HitCharacterSource::Packet, "outgoing");
    hit.damage = 1_000.0;
    hit.target_hp_before = 10_000.0;
    hit.target_hp_after = 9_000.0;
    hit.target_max_hp = 10_000.0;
    set_wire_target(&mut hit, target);

    let (corrections, residuals, _) = decoder.reconcile_current_packet_server_damage_settlements(
        10.05,
        &[server_damage_settlement_for(target, 8_750.0, 0, 1_250)],
        [&hit],
    );

    assert!(residuals.is_empty());
    assert_eq!(corrections.len(), 1);
    assert_eq!(corrections[0].damage, 1_250.0);
    assert_eq!(corrections[0].reconciled_overkill_damage, Some(0.0));
    assert_eq!(
        decoder.server_damage_calibration.client_damage_by_handle[&target],
        1_250.0
    );
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
    let (inferred_follow_ups, corrections, unattributed) =
        decoder.reconcile_boss_hp_updates(10.05, std::slice::from_ref(&update));

    assert!(inferred_follow_ups.is_empty());
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
    let (_, corrections, unattributed) =
        decoder.reconcile_boss_hp_updates(10.05, std::slice::from_ref(&update));

    assert!(unattributed.is_empty());
    assert_eq!(corrections.len(), 1);
    assert_eq!(corrections[0].damage, 1_250.0);
    assert_eq!(corrections[0].reconciled_overkill_damage, Some(0.0));
    assert_eq!(corrections[0].max_hp_reduction, None);
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
        let _ = decoder.observe_server_damage_hit(&hit);
        hits.push(hit);
    }

    let update = boss_hp_update(8_500.0);
    let (_, corrections, unattributed) =
        decoder.reconcile_boss_hp_updates(10.05, std::slice::from_ref(&update));
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
    let (inferred_follow_ups, corrections, unattributed) =
        decoder.reconcile_boss_hp_updates(10.04, std::slice::from_ref(&lethal));

    assert!(inferred_follow_ups.is_empty());
    assert!(corrections.is_empty());
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
    let prepared = decoder.prepare_hits_for_emission(vec![confirmed], &[1051], false, &characters);

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

    let prepared =
        decoder.prepare_hits_for_emission(vec![confirmed, duplicate], &[1051], true, &characters);

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
fn conflicting_wire_target_is_replaced_by_unique_hp_continuity() {
    let mut decoder = PacketDecoder::default();

    let mut small = duplicate_test_hit(9.0, HitCharacterSource::Packet, "outgoing");
    small.target_hp_before = 605_569.0;
    small.target_hp_after = 605_049.0;
    small.target_max_hp = 808_898.0;
    set_wire_target(&mut small, [1; 29]);
    decoder.observe_hit_target(&small);

    let mut large = duplicate_test_hit(9.0, HitCharacterSource::Packet, "outgoing");
    large.target_hp_before = 2_436_602.0;
    large.target_hp_after = 2_435_197.0;
    large.target_max_hp = 2_628_918.0;
    set_wire_target(&mut large, [2; 29]);
    decoder.observe_hit_target(&large);

    let mut mislabeled = duplicate_test_hit(10.0, HitCharacterSource::Packet, "outgoing");
    mislabeled.damage = 1_405.0;
    mislabeled.target_hp_before = 2_435_197.0;
    mislabeled.target_hp_after = 2_433_792.0;
    mislabeled.target_max_hp = 2_628_918.0;
    set_wire_target(&mut mislabeled, [1; 29]);

    decoder.resolve_and_observe_hit_targets(std::slice::from_mut(&mut mislabeled));

    assert_eq!(wire_handle_from_hit(&mislabeled), Some([2; 29]));
}

#[test]
fn partial_stream_defers_session_shadow_even_with_resolved_target() {
    let mut shadow = duplicate_test_hit(10.0, HitCharacterSource::Session, "outgoing");
    set_wire_target(&mut shadow, [1; 29]);

    assert!(should_defer_partial_stream_hit(&shadow));
}

#[test]
fn resolved_reassembled_hit_is_promoted_to_outgoing() {
    let mut hit = duplicate_test_hit(10.0, HitCharacterSource::Session, "unknown");
    set_wire_target(&mut hit, [1; 29]);

    promote_resolved_outgoing_hits(std::slice::from_mut(&mut hit));

    assert_eq!(hit.direction, HitDirection::Outgoing);
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
    let mut ability_catalog =
        AbilityCatalog::load(Path::new(SKILL_DAMAGE_DATA_PATH)).expect("skill table should load");
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
    let mut display_damage = HashMap::<String, (u64, f64)>::new();
    let mut parsed_additional_count = 0_u64;
    let mut parsed_additional_damage = 0_u64;
    let mut capture_stopped = false;
    let mut errors = Vec::new();
    while !handle.is_finished() || !reliable_receiver.is_empty() || !debug_receiver.is_empty() {
        while let Ok(event) = reliable_receiver.try_recv() {
            semantic_events += 1;
            match event {
                EngineEvent::CaptureStopped => capture_stopped = true,
                EngineEvent::Error(error) => errors.push(error),
                EngineEvent::HitFollowUp(follow_up) => {
                    if let Some(attack_type) = follow_up.attack_type
                        && is_authoritative_display_attack_type(&attack_type)
                    {
                        let row = display_damage.entry(attack_type).or_default();
                        row.0 += 1;
                        row.1 += follow_up.damage;
                    }
                }
                EngineEvent::Hit(hit) => {
                    if let Some(attack_type) = hit.attack_type
                        && is_authoritative_display_attack_type(&attack_type)
                    {
                        let row = display_damage.entry(attack_type).or_default();
                        row.0 += 1;
                        row.1 += hit.damage;
                    }
                }
                EngineEvent::HitDamageCorrection(correction) => {
                    if let Some(attack_type) = correction.attack_type
                        && is_authoritative_display_attack_type(&attack_type)
                    {
                        let row = display_damage.entry(attack_type).or_default();
                        row.0 += 1;
                        row.1 += correction.damage;
                    }
                }
                _ => {}
            }
        }
        while let Ok(event) = debug_receiver.try_recv() {
            debug_packets += 1;
            if let EngineEvent::Packet(packet) = event
                && let Some((_, values)) = packet.note.split_once("additional_rows=")
            {
                let mut values = values.split([',', ' ']);
                let packet_additional_count = values
                    .next()
                    .and_then(|value| value.parse::<u64>().ok())
                    .unwrap_or(0);
                let packet_additional_damage = values
                    .find_map(|value| value.strip_prefix("additional_damage="))
                    .and_then(|value| {
                        value
                            .trim_end_matches(|character: char| !character.is_ascii_digit())
                            .parse::<u64>()
                            .ok()
                    })
                    .unwrap_or(0);
                parsed_additional_count += packet_additional_count;
                parsed_additional_damage += packet_additional_damage;
            }
        }
        thread::sleep(Duration::from_millis(1));
    }
    handle.join().expect("pcapng import thread should finish");

    assert!(capture_stopped);
    assert!(errors.is_empty(), "{errors:#?}");
    assert!(semantic_events > 1);
    assert!(debug_packets > 0);
    let display_summary = ["倾陷伤害", "创生花", "覆纹", "浊燃", "黯星", "浸染", "延滞"]
        .into_iter()
        .map(|attack_type| {
            let (count, damage) = display_damage.get(attack_type).copied().unwrap_or_default();
            format!("{attack_type}:{count}/{damage}")
        })
        .collect::<Vec<_>>()
        .join(",");
    println!(
        "large pcapng import completed: semantic_events={semantic_events}, debug_packets={debug_packets}, parsed_additional_count={parsed_additional_count}, parsed_additional_damage={parsed_additional_damage}, display_damage=[{display_summary}], dropped_debug_packets={}",
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

    let prepared = decoder.prepare_hits_for_emission(vec![hit], &[1010, 1020], false, &characters);

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
fn gameplay_effect_character_token_does_not_match_a_longer_name() {
    let characters = HashMap::from([
        (
            1046,
            CharacterInfo {
                name_zh: "男主".to_owned(),
                name_en: "Male".to_owned(),
                color: None,
                avatar: None,
                attribute: None,
            },
        ),
        (
            1051,
            CharacterInfo {
                name_zh: "女主".to_owned(),
                name_en: "Female".to_owned(),
                color: None,
                avatar: None,
                attribute: None,
            },
        ),
    ]);
    let mut hit = targetless_hit();
    hit.char_id = 1046;
    hit.char_source = HitCharacterSource::Session;
    hit.direction = HitDirection::Unknown;
    hit.gameplay_effect_index = Some(1);
    hit.gameplay_effect_name = Some("GE_Player_Female_Skill1_Damage".to_owned());

    assert!(!gameplay_effect_confirms_session_hit(
        &hit,
        &[1046, 1051],
        &characters
    ));
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
    let mut ability_catalog =
        AbilityCatalog::load(Path::new(SKILL_DAMAGE_DATA_PATH)).expect("skill table should load");
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
            hit.timestamp, hit.damage, hit.ability_name, hit.gameplay_effect_name, hit.attack_type
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
    let path = std::env::var("NTE_TEST_CAPTURE_JSON").expect("NTE_TEST_CAPTURE_JSON must be set");
    let (sender, receiver) = unbounded();
    let sender = EngineEventSink::reliable(sender);
    let stop = Arc::new(AtomicBool::new(false));
    let prepared =
        prepare_capture_json_replay(Path::new(&path)).expect("JSON import should be prepared");
    let handle = import_prepared_capture_json(prepared, sender, stop)
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
                    crate::engine::model::AbyssEvent::Location { timestamp, floor } => {
                        (format!("Location floor={floor}"), timestamp)
                    }
                    crate::engine::model::AbyssEvent::RestartDetected { timestamp } => {
                        ("RestartDetected".to_owned(), timestamp)
                    }
                    crate::engine::model::AbyssEvent::RestartHalf { timestamp, half } => {
                        (format!("RestartHalf {half:?}"), timestamp)
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
        "mon_", "Mon_", "boss_", "Boss_", "Abyss_", "abyss_", "Weekly", "weekly", "Wave", "Monster",
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

    let path = std::env::var("NTE_TEST_CAPTURE_JSON").expect("NTE_TEST_CAPTURE_JSON must be set");
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
                                while ident_start > 0 && is_identifier_byte(run[ident_start - 1]) {
                                    ident_start -= 1;
                                }
                                let mut ident_end = needle_at + needle.len();
                                while ident_end < run.len() && is_identifier_byte(run[ident_end]) {
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
                for (variant, needle) in [("raw", handle.as_slice()), ("rev", reversed.as_slice())]
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
                hit.timestamp, hit.direction, hit.bit_shift, hit.offset, hit.handles_in_same_packet,
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

mod special_damage_recording_tests {
    use crate::{
        core::reducer::apply_engine_event,
        engine::{
            capture::{CaptureResources, import_pcapng},
            model::{CombatState, EngineEvent},
            parser::{CHARACTER_DATA_PATH, load_characters},
        },
        storage::{ability_names, i18n::Language},
    };
    use std::{
        path::Path,
        sync::{Arc, atomic::AtomicBool},
    };

    #[test]
    #[ignore = "requires NTE_SPECIAL_DAMAGE_PCAP supplied by the caller"]
    fn supplied_pcap_preserves_malice_damage() {
        let path = std::env::var_os("NTE_SPECIAL_DAMAGE_PCAP").expect("recording path");
        let (catalog, warning) = ability_names::init(Language::SimplifiedChinese);
        assert!(warning.is_none(), "resource load failed");
        let resources = CaptureResources {
            characters: Arc::new(load_characters(Path::new(CHARACTER_DATA_PATH)).unwrap()),
            ability_catalog: catalog,
        };
        let (sender, receiver) = crossbeam_channel::bounded(256);
        let worker = import_pcapng(
            path.into(),
            resources,
            None,
            true,
            true,
            sender,
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        let mut state = CombatState::default();
        while let Ok(event) = receiver.recv() {
            assert!(
                !matches!(&event, EngineEvent::Error(_)),
                "recording replay failed"
            );
            apply_engine_event(&mut state, event);
        }
        worker.join().unwrap();
        let malice: Vec<_> = state
            .hits
            .iter()
            .filter(|h| h.damage_name.as_deref() == Some("恶意"))
            .collect();
        println!(
            "malice_hits={} malice_damage={}",
            malice.len(),
            malice.iter().map(|h| h.damage).sum::<f64>()
        );
        assert!(
            !malice.is_empty(),
            "the supplied recording must retain type-31 damage"
        );
    }
}
