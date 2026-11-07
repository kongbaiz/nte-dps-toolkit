use std::{fs, time::SystemTime};

use super::*;
use nte_dps_tool::core::history::prepare_history_archive;
use nte_dps_tool::core::hud::HudDataState;
use nte_dps_tool::storage::config::{HudModule, MainDpsAttribution, MainDpsMetric};

#[test]
fn stream_delivery_waits_for_exact_owner_generation_and_sequence_ack() {
    let registry = StreamRegistry::default();
    let stream = registry.reserve("console", "timeline:flow").unwrap();
    assert!(registry.activate(&stream).unwrap());
    assert!(stream.begin_delivery());
    for _ in 0..1000 {
        assert!(!stream.delivery_ready());
        assert!(!stream.begin_delivery());
    }
    for (owner, generation, sequence) in [
        ("main-dps", stream.generation(), 1),
        ("console", stream.generation() + 1, 1),
        ("console", stream.generation(), 2),
    ] {
        assert!(
            !registry
                .acknowledge(owner, "timeline:flow", generation, sequence)
                .unwrap()
        );
        assert!(!stream.delivery_ready());
    }
    assert!(
        registry
            .acknowledge("console", "timeline:flow", stream.generation(), 1)
            .unwrap()
    );
    assert!(stream.delivery_ready());
    assert!(stream.begin_delivery());
    assert!(
        !registry
            .acknowledge("console", "timeline:flow", stream.generation(), 1)
            .unwrap()
    );
    assert!(!stream.delivery_ready());
    assert!(
        registry
            .acknowledge("console", "timeline:flow", stream.generation(), 2)
            .unwrap()
    );
    assert!(stream.delivery_ready());
}

#[test]
fn stream_delivery_old_ack_cannot_release_replacement_and_stop_remains_idempotent() {
    let registry = StreamRegistry::default();
    let old = registry.reserve("console", "timeline:flow").unwrap();
    registry.activate(&old).unwrap();
    assert!(old.begin_delivery());
    let next = registry.reserve("console", "timeline:flow").unwrap();
    registry.activate(&next).unwrap();
    assert!(!old.delivery_ready());
    assert!(!old.begin_delivery());
    assert!(next.begin_delivery());
    assert!(
        !registry
            .acknowledge("console", "timeline:flow", old.generation(), 1)
            .unwrap()
    );
    assert!(!next.delivery_ready());
    assert!(
        registry
            .acknowledge("console", "timeline:flow", next.generation(), 1)
            .unwrap()
    );
    assert!(registry.stop_for_window("console").unwrap() > 0);
    assert_eq!(registry.stop_for_window("console").unwrap(), 0);
    assert!(!next.begin_delivery());
    assert!(
        !registry
            .acknowledge("console", "timeline:flow", next.generation(), 1)
            .unwrap()
    );
    assert_eq!(registry.len().unwrap(), 0);
}

#[test]
#[ignore = "requires NTE_TEST_AUTO_CAPTURE and NTE_TEST_EXPECTED_DAMAGE real-capture inputs"]
fn automatic_midstream_capture_reaches_main_dps_display_contract() {
    assert!(
        std::env::var_os("NTE_EXACT_PACKET_CONFIG").is_none(),
        "test must exercise default startup"
    );
    let path = PathBuf::from(std::env::var_os("NTE_TEST_AUTO_CAPTURE").expect("capture input"));
    let expected: f64 = std::env::var("NTE_TEST_EXPECTED_DAMAGE")
        .expect("expected total")
        .parse()
        .unwrap();
    let mut expected_display = expected;
    let mut expected_shared = std::env::var("NTE_TEST_EXPECTED_SHARED_DAMAGE")
        .ok()
        .map(|v| v.parse::<f64>().unwrap());
    let display_half = match std::env::var("NTE_TEST_ABYSS_HALF").ok().as_deref() {
        None | Some("first") => AbyssHalf::First,
        Some("second") => AbyssHalf::Second,
        Some(_) => panic!("unsupported acceptance half"),
    };
    let mut expected_reduction = std::env::var("NTE_TEST_EXPECTED_MAX_HP_REDUCTION")
        .ok()
        .map(|v| v.parse::<f64>().unwrap());
    let mut expected_reduction_rows =
        std::env::var("NTE_TEST_EXPECTED_HP_ROWS").map_or(23, |v| v.parse::<usize>().unwrap());
    let mut expected_extra = std::env::var("NTE_TEST_EXPECTED_HP_EXTRA")
        .map_or(233351.125, |v| v.parse::<f64>().unwrap());
    let expected_assist = std::env::var("NTE_TEST_EXPECTED_ASSIST_DAMAGE")
        .ok()
        .map(|v| v.parse::<f64>().unwrap());
    let (resources, warnings) =
        LiveCaptureResources::load(nte_dps_tool::storage::i18n::Language::SimplifiedChinese);
    println!(
        "ACCEPTANCE_RESOURCES characters={} warnings={}",
        resources.characters.len(),
        warnings.len()
    );
    let service = LiveCaptureService::new(resources);
    let state = AppState::new(UiConfig::default(), service.clone());
    let before = state.main_dps_stream_revision().unwrap();
    let empty = crate::contract::main_dps::MainDpsSnapshot::from_state(&state).unwrap();
    assert_eq!(empty.readout.summary.total_damage, 0.0);
    service
        .request_replay(CaptureReplayKind::Pcapng, path, None, true, false)
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(300);
    while service.replay_running().unwrap() {
        assert!(Instant::now() < deadline, "replay did not terminate");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_ne!(service.status().phase, LiveCapturePhase::Failed);
    println!(
        "ACCEPTANCE_STATE {:?} {:?}",
        service.status(),
        service
            .with_state(|s| (s.hits.len(), s.total_damage, s.total_damage_taken))
            .unwrap()
    );
    let mut archives = service.take_pending_abyss_archives().unwrap();
    if !archives.is_empty() {
        // CaptureStopped legitimately archives an Abyss round and leaves
        // an empty live row. Verify the actual archived record through the
        // History presentation path, not a fabricated nonempty live row.
        assert_eq!(
            archives.len(),
            1,
            "this acceptance input must contain one archived round"
        );
        let archive = archives.pop().unwrap();
        let basis = if archive.dps_time_mode.subtracts_time_stop() {
            DpsTimeMode::TimeStopAdjusted
        } else {
            DpsTimeMode::RealTime
        };
        let encoded = serde_json::to_vec(&archive.details).unwrap();
        let details: HistoryCombatDetails = serde_json::from_slice(&encoded).unwrap();
        let archived = details.into_combat_state();
        assert_eq!(
            archived.total_damage, expected,
            "complete archived capture total"
        );
        if let Some(reduction) = expected_reduction {
            assert_eq!(archived.max_hp_reduction, reduction);
            assert_eq!(
                archived
                    .hits
                    .iter()
                    .filter(|h| h.max_hp_reduction > 0.0)
                    .count(),
                expected_reduction_rows
            );
            assert_eq!(
                archived
                    .hits
                    .iter()
                    .map(|h| h.follow_up_damage)
                    .sum::<f64>(),
                expected_extra
            );
        }
        if let Some(expected) = expected_assist {
            let assist: Vec<_> = archived
                .hits
                .iter()
                .filter(|h| h.gameplay_effect_index == Some(6114))
                .collect();
            assert_eq!(assist.iter().map(|h| h.damage).sum::<f64>(), expected);
            assert!(assist.iter().all(|h| h.char_id == 1042));
            use nte_dps_tool::core::{
                skills::{SkillsProjectionOptions, project_skills},
                timeline::TimelineScope,
            };
            let skills = project_skills(
                &archived,
                &HashMap::new(),
                SkillsProjectionOptions {
                    scope: TimelineScope::Whole,
                    language: nte_dps_tool::storage::i18n::Language::SimplifiedChinese,
                },
            );
            let skills = crate::contract::skills::SkillsSnapshot::from_projection(
                skills,
                1,
                TimelineScope::Whole,
            );
            for (effect, expected_name) in [
                (6114, "失乐鸟·协同攻击"),
                (3241, "番茄酱盛宴·失谐强化追加伤害"),
                (6094, "鸫歌·黯星强化结算"),
                (1349, "墨菲克斯·弹丸回击"),
            ] {
                let rows: Vec<_> = skills
                    .rows
                    .iter()
                    .filter(|r| r.gameplay_effect_index == Some(effect))
                    .collect();
                assert!(!rows.is_empty(), "missing effect {effect}");
                assert!(
                    rows.iter().all(|r| r.name == expected_name),
                    "wrong label: {rows:?}"
                );
            }
            println!("SKILL_NAMES_PASS assist_damage={expected}");
        }
        if let Some(expected) = expected_shared {
            assert_eq!(
                archived.damage_attribution_summary().shared_damage,
                expected
            );
            let shared = archived.indexed_combat_details(
                None,
                &nte_dps_tool::engine::model::IndexedCombatDetailFilter::SharedMechanics,
                None,
                0,
                100,
            );
            assert_eq!(shared.total_hits, 2);
            assert_eq!(shared.total_damage, expected);
            assert_eq!(
                shared.rows[0].1.damage_component.as_deref(),
                Some("普通倾陷伤害")
            );
            assert_eq!(
                shared.rows[1].1.damage_component.as_deref(),
                Some("达芙蒂尔·额外倾陷伤害")
            );
            assert_eq!(
                archived
                    .hits
                    .iter()
                    .filter(|h| h.gameplay_effect_index == Some(3311))
                    .map(|h| h.damage)
                    .sum::<f64>(),
                6773.0
            );
        }
        if archived.abyss.is_active() {
            // Main DPS presents the selected/current half. Independently
            // sum its retained rows rather than expecting the whole-file
            // total in a half-filtered display.
            let half = archived.abyss.half(display_half);
            if expected_shared.is_some() {
                expected_shared = Some(half.damage_attribution_summary().shared_damage);
            }
            expected_display = half
                .hits
                .iter()
                .filter(|h| !h.direction.is_incoming())
                .map(|h| h.total_damage())
                .sum();
            if expected_reduction.is_some() {
                expected_reduction = Some(half.hits.iter().map(|h| h.max_hp_reduction).sum());
                expected_reduction_rows = half
                    .hits
                    .iter()
                    .filter(|h| h.max_hp_reduction > 0.0)
                    .count();
                expected_extra = half.hits.iter().map(|h| h.follow_up_damage).sum();
            }
        }
        println!(
            "ACCEPTANCE_ARCHIVE hits={} damage={}",
            archived.hits.len(),
            archived.total_damage
        );
        let id = "acceptance-recorded-round".to_owned();
        *state.0.history.round_cache.lock().unwrap() = MainRoundCache {
            revision: Some(state.history_revision()),
            index: Arc::new(vec![HistoryRoundIndex {
                id: id.clone(),
                display_time: "Recorded capture".into(),
                abyss_floor: archived.abyss.floor,
                has_details: true,
            }]),
        };
        let operation = state.reserve_main_round_selection();
        state
            .set_main_selected_round_id_with(Some(id), operation, |_| Ok((archived, basis)))
            .unwrap();
        state
            .set_main_selected_abyss_half(Some(display_half))
            .unwrap();
    }
    let snapshot = crate::contract::main_dps::MainDpsSnapshot::from_state(&state).unwrap();
    assert_eq!(snapshot.readout.summary.total_damage, expected_display);
    if let Some(expected) = expected_shared {
        assert_eq!(snapshot.readout.damage_attribution.shared_damage, expected);
        state
            .set_main_dps_detail_request(
                MainDpsDetailKind::Team,
                MainDpsDetailRequest {
                    character_id: None,
                    filter: CombatDetailFilter::SharedMechanics,
                    skill_filter: None,
                },
            )
            .unwrap();
        let detail = crate::contract::main_dps_detail::MainDpsDetailSnapshot::from_state(
            &state,
            MainDpsDetailKind::Team,
            0,
            100,
        )
        .unwrap();
        assert_eq!(detail.total_damage, expected);
        assert_eq!(detail.rows.len(), 2);
        assert!(detail.rows[0].skill.contains("普通倾陷伤害"));
        assert!(detail.rows[1].skill.contains("达芙蒂尔·额外倾陷伤害"));
        state
            .set_main_dps_detail_request(MainDpsDetailKind::Team, MainDpsDetailRequest::default())
            .unwrap();
        let all = crate::contract::main_dps_detail::MainDpsDetailSnapshot::from_state(
            &state,
            MainDpsDetailKind::Team,
            0,
            500,
        )
        .unwrap();
        assert!(
            all.rows
                .iter()
                .any(|r| r.skill.contains("漆黑青春妄想·黑之书"))
        );
        println!(
            "SHARED_BREAK_PASS damage={expected} ordinary_and_extra_labels_distinct=true black_book_named=true"
        );
    }
    if let Some(reduction) = expected_reduction {
        assert_eq!(
            snapshot.readout.damage_attribution.max_hp_reduction,
            reduction
        );
        let mut offset = 0;
        let mut reduction_rows = 0;
        let mut reductions = 0.0;
        let mut extra = 0.0;
        loop {
            let detail = crate::contract::main_dps_detail::MainDpsDetailSnapshot::from_state(
                &state,
                MainDpsDetailKind::Team,
                offset,
                200,
            )
            .unwrap();
            for row in &detail.rows {
                if let Some(value) = row.max_hp_reduction
                    && value > 0.0
                {
                    reduction_rows += 1;
                    reductions += value;
                }
                extra += row.follow_up_damage;
                if [1036, 1042].contains(&row.character_id) {
                    assert!(
                        !row.skill.contains("Unmapped") && !row.skill.contains("未映射"),
                        "mechanic label not projected"
                    );
                }
            }
            offset += detail.rows.len();
            if offset >= detail.total_hits {
                break;
            }
            assert!(!detail.rows.is_empty());
        }
        assert_eq!(reductions, reduction);
        assert_eq!(reduction_rows, expected_reduction_rows);
        assert_eq!(extra, expected_extra);
        println!(
            "MECHANIC_DETAIL_PASS reduction_rows={reduction_rows} max_hp_reduction={reductions} additional_hp_loss={extra}"
        );
    }
    assert_eq!(
        crate::contract::main_dps::MainDpsSnapshot::from_state(&state)
            .unwrap()
            .readout
            .summary
            .total_damage,
        expected_display,
        "an idle reread must retain the refreshed projection"
    );
    assert!(!snapshot.readout.characters.is_empty());
    assert!(snapshot.has_live_session_data || snapshot.selected_round_id.is_some());
    assert_ne!(state.main_dps_stream_revision().unwrap(), before);
    if let Some(path) = std::env::var_os("NTE_TEST_MAIN_SNAPSHOT_OUTPUT") {
        let bytes = serde_json::to_vec_pretty(&snapshot).unwrap();
        let mut output = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .unwrap();
        std::io::Write::write_all(&mut output, &bytes).unwrap();
    }
    println!(
        "AUTO_MAIN_DPS_PASS total={} character_rows={} data_state={}",
        expected_display,
        snapshot.readout.characters.len(),
        snapshot.readout.data_state
    );
}

#[test]
fn desktop_capture_retains_packets_for_the_live_inspector() {
    assert_eq!(DESKTOP_PACKET_EMISSION_MODE, PacketEmissionMode::FullDebug);
}

#[test]
fn capture_export_rejects_same_length_hits_from_a_replacement_session() {
    let live_capture = LiveCaptureService::new(LiveCaptureResources::default());
    let mut original = CombatState::default();
    original.push_hit(test_hit(100.0));
    live_capture
        .restore_session(original, CaptureQualitySource::JsonReplay)
        .expect("original capture session should install");
    let state = AppState::new(UiConfig::default(), live_capture.clone());
    let (plan, session_generation) = state
        .diagnostics_capture_export_plan()
        .expect("original export plan should be available");

    let mut replacement = CombatState::default();
    replacement.push_hit(test_hit(999.0));
    assert_eq!(
        replacement.hits_generation,
        plan.hits_generation(),
        "fixture proves per-state hit generations can collide"
    );
    live_capture
        .restore_session(replacement, CaptureQualitySource::JsonReplay)
        .expect("replacement capture session should install");

    let error = state
        .diagnostics_capture_export_hit_page(
            session_generation,
            plan.hits_generation(),
            plan.hit_count(),
            0,
            1,
        )
        .expect_err("a replacement session must invalidate the export plan");
    assert_eq!(error.code, CoreErrorCode::CaptureStateUnavailable);
    assert!(error.detail.contains("source changed"));
}

fn test_hit(damage: f64) -> nte_dps_tool::engine::model::Hit {
    use nte_dps_tool::engine::model::{Hit, HitCharacterSource, HitDirection};

    Hit {
        timestamp: 1.0,
        char_id: 7,
        char_name: "Fixture".to_owned(),
        char_known: true,
        damage,
        byte_offset: 0,
        bit_shift: 0,
        char_source: HitCharacterSource::Packet,
        direction: HitDirection::Outgoing,
        target_hp_before: 1_000.0,
        target_hp_after: 1_000.0 - damage,
        target_max_hp: 1_000.0,
        max_hp_reduction: 0.0,
        target_hp_percent: 50.0,
        target_id: None,
        target_name: None,
        target_name_en: None,
        target_name_ja: None,
        target_monster_id: None,
        target_context: Vec::new(),
        gameplay_effect_index: Some(17),
        gameplay_effect_name: Some("GE_Fixture".to_owned()),
        ability_name: Some("GA_Fixture".to_owned()),
        damage_name: Some("Fixture Damage".to_owned()),
        damage_component: None,
        attack_type: Some("Skill".to_owned()),
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

#[test]
fn plugin_control_reservation_excludes_mode_switch_and_releases_after_failure() {
    use nte_dps_tool::core::toolkit::DataMode;
    let path = temporary_config_path("plugin_control_reservation");
    let state = AppState::new_with_config_path(
        UiConfig::default(),
        LiveCaptureService::new(LiveCaptureResources::default()),
        path.clone(),
    );
    state.set_data_mode(DataMode::Plugin).unwrap();
    {
        let _guard = state.reserve_plugin_control().unwrap();
        assert!(state.reserve_plugin_control().is_err());
        assert!(state.set_data_mode(DataMode::PacketCapture).is_err());
        assert_eq!(state.data_mode(), DataMode::Plugin);
    }
    state.set_data_mode(DataMode::PacketCapture).unwrap();
    assert_eq!(state.data_mode(), DataMode::PacketCapture);
    let _ = fs::remove_file(path);
}

#[test]
fn data_modes_default_switch_noop_and_preserve_records() {
    use nte_dps_tool::core::toolkit::DataMode;
    let path = temporary_config_path("data_modes");
    let live = LiveCaptureService::new(LiveCaptureResources::default());
    let state = AppState::new_with_config_path(UiConfig::default(), live.clone(), path.clone());
    assert_eq!(state.data_mode(), DataMode::PacketCapture);
    assert_eq!(
        state.main_presented_dps_time_mode().unwrap(),
        DpsTimeMode::RealTime
    );
    let before = state.0.presentation.revision.load(Ordering::Acquire);
    state.set_data_mode(DataMode::PacketCapture).unwrap();
    assert_eq!(
        state.0.presentation.revision.load(Ordering::Acquire),
        before
    );
    state.set_data_mode(DataMode::Plugin).unwrap();
    assert_eq!(state.data_mode(), DataMode::Plugin);
    assert_eq!(
        state.main_presented_dps_time_mode().unwrap(),
        DpsTimeMode::TimeStopAdjusted
    );
    assert_eq!(
        live.history_archive_policy().requested_dps_time_mode,
        DpsTimeBasis::SubtractTimeStop
    );
    assert_eq!(live.status().phase, LiveCapturePhase::Idle);
    assert!(state.0.presentation.revision.load(Ordering::Acquire) > before);
    let saved: UiConfig = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(saved.data_mode, DataMode::Plugin);
    let selection = state.reserve_main_round_selection();
    state
        .set_main_selected_round_id_with(Some("recorded-plugin".into()), selection, |_| {
            Ok((CombatState::default(), DpsTimeMode::TimeStopAdjusted))
        })
        .unwrap();
    state.set_data_mode(DataMode::PacketCapture).unwrap();
    assert_eq!(state.data_mode(), DataMode::PacketCapture);
    assert_eq!(
        live.history_archive_policy().requested_dps_time_mode,
        DpsTimeBasis::WallClock
    );
    assert_eq!(
        state.main_presented_dps_time_mode().unwrap(),
        DpsTimeMode::TimeStopAdjusted,
        "a stored round keeps its original time basis across a source switch"
    );
    println!(
        "MODE_PASS: default packet_capture; plugin switch; no-op revision stable; no automatic source start"
    );
    fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

fn temporary_config_path(tag: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!(
        "nte_tauri_hud_{tag}_{}_{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("system clock after epoch")
            .as_nanos()
    ));
    fs::create_dir_all(&directory).expect("temporary config directory");
    directory.join("config.json")
}

fn temporary_history_tombstone(
    tag: &str,
) -> (
    PathBuf,
    nte_dps_tool::storage::history::HistoryDeleteTombstone,
) {
    use nte_dps_tool::storage::history::{save_summary_to_dir, tombstone_record_from_dir};

    let directory = std::env::temp_dir().join(format!(
        "nte-tauri-history-undo-{tag}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time after epoch")
            .as_nanos()
    ));
    let record = save_summary_to_dir(
        &directory,
        nte_dps_tool::engine::model::CombatSessionSummary::default(),
    )
    .expect("save History tombstone fixture");
    let tombstone = tombstone_record_from_dir(&directory, &record.id)
        .expect("create History tombstone fixture")
        .expect("History fixture exists");
    (directory, tombstone)
}

#[test]
fn replacing_subscription_stops_previous_stream() {
    let state = AppState::default();
    let previous = state
        .reserve_stream("hud", "technical")
        .expect("reserve previous stream");
    assert!(
        state
            .activate_stream(&previous)
            .expect("activate previous stream")
    );
    let current = state
        .reserve_stream("hud", "technical")
        .expect("reserve current stream");
    assert!(
        state
            .activate_stream(&current)
            .expect("activate current stream")
    );

    assert!(previous.is_cancelled());
    assert!(!current.is_cancelled());
}

#[test]
fn newer_stream_generation_rejects_a_stale_late_activation() {
    let state = AppState::default();
    let stale = state
        .reserve_stream("hud", "technical")
        .expect("reserve stale stream generation");
    let current = state
        .reserve_stream("hud", "technical")
        .expect("reserve current stream generation");

    assert!(
        state
            .activate_stream(&current)
            .expect("activate current stream")
    );
    assert!(!state.activate_stream(&stale).expect("reject stale stream"));
    assert!(stale.is_cancelled());
    assert!(!current.is_cancelled());
}

#[test]
fn full_registry_allows_same_identity_replacement_without_dropping_current() {
    let state = AppState::default();
    let current = state
        .reserve_stream("hud", "technical")
        .expect("reserve current stream");
    assert!(
        state
            .activate_stream(&current)
            .expect("activate current stream")
    );
    for index in 1..MAX_ACTIVE_STREAMS {
        let pending = state
            .reserve_stream("hud", &format!("technical:{index}"))
            .expect("fill bounded stream registry");
        assert!(
            state
                .activate_stream(&pending)
                .expect("activate filler stream")
        );
    }

    assert!(state.reserve_stream("hud", "overflow").is_err());
    let replacement = state
        .reserve_stream("hud", "technical")
        .expect("same identity must reuse its registry capacity");
    assert!(
        state
            .activate_stream(&replacement)
            .expect("activate replacement")
    );
    assert!(current.is_cancelled());
    assert!(!replacement.is_cancelled());
}

#[test]
fn pending_replacement_reservations_are_globally_bounded() {
    let state = AppState::default();
    let pending = (0..MAX_PENDING_STREAMS)
        .map(|_| {
            state
                .reserve_stream("hud", "technical")
                .expect("reserve bounded pending replacement")
        })
        .collect::<Vec<_>>();

    assert_eq!(pending.len(), MAX_PENDING_STREAMS);
    assert_eq!(
        state.reserve_stream("hud", "technical").err(),
        Some(StreamRegistryError::CapacityUnavailable)
    );
    assert_eq!(state.shutdown_streams().expect("cancel pending streams"), 1);
    assert!(pending.iter().all(StreamRegistration::is_cancelled));
}

#[test]
fn poisoned_stream_registry_resets_atomically_and_fails_the_triggering_operation() {
    let state = AppState::default();
    let current = state
        .reserve_stream("hud", "technical")
        .expect("reserve current stream");
    assert!(
        state
            .activate_stream(&current)
            .expect("activate current stream")
    );
    let poison_state = state.clone();
    let _ = std::thread::spawn(move || {
        let _guard = poison_state
            .0
            .streams
            .entries
            .lock()
            .expect("healthy stream registry lock");
        panic!("poison stream registry");
    })
    .join();

    assert_eq!(
        state.reserve_stream("hud", "next").err(),
        Some(StreamRegistryError::RuntimeUnavailable)
    );
    assert!(current.is_cancelled());
    assert_eq!(state.stream_registry_len().expect("reset registry"), 0);

    let recovered = state
        .reserve_stream("hud", "next")
        .expect("reserve after controlled reset");
    assert!(
        state
            .activate_stream(&recovered)
            .expect("activate recovered stream")
    );
}

#[test]
fn stream_identity_includes_owner_and_wrong_owner_stop_is_a_no_op() {
    let state = AppState::default();
    let hud = state
        .reserve_stream("hud", "shared:key")
        .expect("reserve HUD stream");
    let console = state
        .reserve_stream("console", "shared:key")
        .expect("reserve Console stream");
    assert!(state.activate_stream(&hud).expect("activate HUD stream"));
    assert!(
        state
            .activate_stream(&console)
            .expect("activate Console stream")
    );
    let revision = state.stream_revision();

    assert!(
        !state
            .stop_stream("main-dps", "shared:key")
            .expect("wrong owner no-op")
    );
    assert_eq!(state.stream_revision(), revision);
    assert!(!hud.is_cancelled());
    assert!(!console.is_cancelled());
    assert!(
        state
            .stop_stream("hud", "shared:key")
            .expect("stop HUD stream")
    );
    assert!(hud.is_cancelled());
    assert!(!console.is_cancelled());
}

#[test]
fn stream_registry_rejects_unbounded_or_non_ascii_identities() {
    let state = AppState::default();

    assert_eq!(
        state.reserve_stream("", "technical").err(),
        Some(StreamRegistryError::InvalidIdentity)
    );
    assert_eq!(
        state.reserve_stream("hud", "technical/path").err(),
        Some(StreamRegistryError::InvalidIdentity)
    );
    assert_eq!(
        state.reserve_stream("hud", &"a".repeat(129)).err(),
        Some(StreamRegistryError::InvalidIdentity)
    );
}

#[test]
fn stream_shutdown_is_idempotent_and_cancels_current_and_pending_tokens() {
    let state = AppState::default();
    let current = state
        .reserve_stream("hud", "technical")
        .expect("reserve current stream");
    assert!(
        state
            .activate_stream(&current)
            .expect("activate current stream")
    );
    let pending = state
        .reserve_stream("hud", "technical")
        .expect("reserve replacement stream");

    assert_eq!(state.shutdown_streams().expect("shutdown streams"), 1);
    assert!(current.is_cancelled());
    assert!(pending.is_cancelled());
    assert_eq!(state.shutdown_streams().expect("duplicate shutdown"), 0);
    assert_eq!(state.stream_registry_len().expect("empty registry"), 0);
}

#[test]
fn late_starting_history_worker_cannot_overwrite_a_newer_intent() {
    let state = AppState::default();
    let slow_operation = state.reserve_main_round_selection();
    let newest_operation = state.reserve_main_round_selection();

    assert!(
        state
            .set_main_selected_round_id_with(Some("newest".to_owned()), newest_operation, |_| {
                let mut selected = CombatState::default();
                selected.push_hit(test_hit(200.0));
                Ok((selected, DpsTimeMode::RealTime))
            },)
            .expect("commit newer History selection")
    );
    let stale_loader_ran = AtomicBool::new(false);
    assert!(
        !state
            .set_main_selected_round_id_with(Some("slow".to_owned()), slow_operation, |_| {
                stale_loader_ran.store(true, Ordering::Release);
                let mut selected = CombatState::default();
                selected.push_hit(test_hit(100.0));
                Ok((selected, DpsTimeMode::RealTime))
            },)
            .expect("superseded History selection")
    );
    assert!(!stale_loader_ran.load(Ordering::Acquire));

    assert_eq!(state.main_selected_round_id().as_deref(), Some("newest"));
    assert_eq!(
        state
            .with_main_presented_state(|selected| selected.total_damage)
            .expect("project latest History selection"),
        200.0
    );
}

#[test]
fn selected_history_round_reloads_same_id_after_history_revision_changes() {
    let state = AppState::default();
    let first_operation = state.reserve_main_round_selection();
    assert!(
        state
            .set_main_selected_round_id_with(Some("reimported".to_owned()), first_operation, |_| {
                let mut selected = CombatState::default();
                selected.push_hit(test_hit(100.0));
                Ok((selected, DpsTimeMode::RealTime))
            },)
            .expect("select first History contents")
    );

    state.bump_history_revision();
    assert!(
        state.presentation_mode_snapshot().selected_round.is_none(),
        "a History mutation must not keep projecting materialized old contents"
    );
    let second_operation = state.reserve_main_round_selection();
    assert!(
        state
            .set_main_selected_round_id_with(
                Some("reimported".to_owned()),
                second_operation,
                |_| {
                    let mut selected = CombatState::default();
                    selected.push_hit(test_hit(250.0));
                    Ok((selected, DpsTimeMode::RealTime))
                },
            )
            .expect("reload replaced History contents")
    );
    assert_eq!(
        state
            .with_main_presented_state(|selected| selected.total_damage)
            .expect("project replaced History contents"),
        250.0
    );
}

#[test]
fn already_loading_history_selection_cannot_overwrite_a_newer_intent() {
    let state = AppState::default();
    let slow_operation = state.reserve_main_round_selection();
    let slow_state = state.clone();
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let slow = std::thread::spawn(move || {
        slow_state.set_main_selected_round_id_with(Some("slow".to_owned()), slow_operation, |_| {
            entered_tx.send(()).expect("signal slow History load");
            release_rx.recv().expect("release slow History load");
            let mut selected = CombatState::default();
            selected.push_hit(test_hit(100.0));
            Ok((selected, DpsTimeMode::RealTime))
        })
    });
    entered_rx.recv().expect("slow History load started");
    let newest_operation = state.reserve_main_round_selection();
    assert!(
        state
            .set_main_selected_round_id_with(Some("newest".to_owned()), newest_operation, |_| {
                let mut selected = CombatState::default();
                selected.push_hit(test_hit(200.0));
                Ok((selected, DpsTimeMode::RealTime))
            },)
            .expect("commit newer History selection")
    );
    release_tx.send(()).expect("finish slow History load");
    assert!(
        !slow
            .join()
            .expect("join slow History selection")
            .expect("superseded History selection")
    );
    assert_eq!(state.main_selected_round_id().as_deref(), Some("newest"));
}

#[test]
fn timeline_projection_cache_reuses_one_revision_across_consumers() {
    let state = AppState::default();
    let first = state
        .timeline_projection(TimelineScope::Whole)
        .expect("first projection");
    let second = state
        .timeline_projection(TimelineScope::Whole)
        .expect("same-revision projection");
    let other_scope = state
        .timeline_projection(TimelineScope::First)
        .expect("other-scope projection");

    assert!(Arc::ptr_eq(&first, &second));
    assert!(!Arc::ptr_eq(&first, &other_scope));
    assert!(first.buckets.len() <= MAX_TIMELINE_BUCKETS);
}

#[test]
fn main_readout_cache_reuses_exact_revision_and_invalidates_on_hit() {
    let state = AppState::default();

    let first = state.main_dps_readout().expect("first main readout");
    let second = state
        .main_dps_readout()
        .expect("same-revision main readout");
    assert_eq!(first.hud, second.hud);

    let mut combat = CombatState::default();
    combat.push_hit(test_hit(250.0));
    state.restore_live_state_for_test(combat, CaptureQualitySource::Live);
    let changed = state
        .main_dps_readout()
        .expect("new-hit main readout projection");
    assert_eq!(
        changed.hud.summary.expect("live summary").total_damage,
        250.0
    );

    // Drive more exact revisions than the cache can retain. Each snapshot
    // remains independently usable, while old revisions are evicted.
    for damage in 1..=8 {
        let mut combat = CombatState::default();
        combat.push_hit(test_hit(f64::from(damage)));
        state.restore_live_state_for_test(combat, CaptureQualitySource::Live);
        let readout = state
            .main_dps_readout()
            .expect("bounded-cache main readout projection");
        assert_eq!(
            readout
                .hud
                .summary
                .expect("bounded-cache summary")
                .total_damage,
            f64::from(damage)
        );
    }
    let (cache, recovered) = state.0.presentation.lock_main_readout_cache();
    assert!(!recovered);
    assert_eq!(cache.len(), 6, "readout cache memory must remain bounded");
}

#[test]
fn main_readout_can_include_max_hp_reduction_in_total_and_denominator() {
    let config_path = temporary_config_path("max_hp_reduction_total");
    let config = UiConfig {
        include_max_hp_reduction_in_total_damage: true,
        ..UiConfig::default()
    };
    let state = AppState::new_with_config_path(
        config,
        LiveCaptureService::new(LiveCaptureResources::default()),
        config_path.clone(),
    );
    let mut hit = test_hit(100.0);
    hit.max_hp_reduction = 50.0;
    let mut combat = CombatState::default();
    combat.push_hit(hit);
    state.restore_live_state_for_test(combat, CaptureQualitySource::Live);

    let readout = state.main_dps_readout().expect("combined main readout");
    let summary = readout.hud.summary.expect("combined summary");
    assert_eq!(summary.total_damage, 150.0);
    assert_eq!(summary.team_dps, 150.0);
    assert_eq!(readout.damage_attribution.total_damage, 150.0);
    assert_eq!(readout.damage_attribution.max_hp_reduction, 50.0);
    assert!((readout.hud.characters[0].damage_share_percent - 66.666_666).abs() < 0.001);

    fs::remove_dir_all(config_path.parent().expect("config parent"))
        .expect("remove temporary config");
}

#[test]
fn plugin_equipment_does_not_replace_imported_replay_inventory() {
    let state = AppState::new(
        UiConfig {
            data_mode: nte_dps_tool::storage::config::DataMode::Plugin,
            ..UiConfig::default()
        },
        LiveCaptureService::new(LiveCaptureResources::default()),
    );
    assert!(state.uses_plugin_equipment().unwrap());
    for source in [
        CaptureQualitySource::JsonReplay,
        CaptureQualitySource::PcapngReplay,
    ] {
        state.restore_live_state_for_test(CombatState::default(), source);
        assert!(!state.uses_plugin_equipment().unwrap());
    }
    state.restore_live_state_for_test(CombatState::default(), CaptureQualitySource::Plugin);
    assert!(state.uses_plugin_equipment().unwrap());
}

#[test]
fn empty_curtain_data_snapshot_is_detached_from_live_state() {
    use nte_dps_tool::engine::model::{EmptyCurtainItem, HtItemNetId};

    let state = AppState::default();
    let mut combat = CombatState::default();
    combat.empty_curtain.push(EmptyCurtainItem {
        id: HtItemNetId { solt: 1, serial: 2 },
        item_id: "fixture-item".to_owned(),
        level: 1,
        main_stats: Vec::new(),
        sub_stats: Vec::new(),
        locked: false,
        discarded: false,
        character_net_id: None,
        equipped_character_id: None,
        equipped_placement: None,
    });
    state.restore_live_state_for_test(combat, CaptureQualitySource::Live);

    let (items, characters, _) = state
        .empty_curtain_data_snapshot()
        .expect("healthy live-capture empty-curtain snapshot");
    state.restore_live_state_for_test(CombatState::default(), CaptureQualitySource::Live);

    assert_eq!(items.len(), 1);
    assert_eq!(items[0].item_id, "fixture-item");
    assert!(characters.is_empty());
}

#[test]
fn pause_freezes_the_presented_state_until_resume() {
    let config_path = temporary_config_path("pause_freezes_projection");
    let live_capture = LiveCaptureService::new(LiveCaptureResources::default());
    let mut first = CombatState::default();
    first.push_hit(test_hit(125.0));
    live_capture
        .restore_session(first, CaptureQualitySource::Live)
        .expect("healthy live-capture state");
    let state = AppState::new_with_config_path(
        UiConfig::default(),
        live_capture.clone(),
        config_path.clone(),
    );

    state
        .set_main_processing_paused(true)
        .expect("pause healthy live capture");
    let paused_revision = state
        .main_dps_stream_revision()
        .expect("healthy live-capture revision");
    let mut second = CombatState::default();
    second.push_hit(test_hit(300.0));
    second.push_hit(test_hit(25.0));
    live_capture
        .restore_session(second, CaptureQualitySource::Plugin)
        .expect("healthy live-capture state");

    assert_eq!(
        state.main_presented_dps_time_mode().unwrap(),
        DpsTimeMode::RealTime,
        "paused packet data retains its clock policy after replacing the live source"
    );

    assert_eq!(
        state
            .with_main_presented_state(|presented| presented.total_damage)
            .expect("healthy paused presentation"),
        125.0
    );
    assert!(
        state
            .main_paused_event_counts()
            .expect("healthy paused event counts")
            .0
            > 0
    );
    assert_ne!(
        state
            .main_dps_stream_revision()
            .expect("healthy live-capture revision"),
        paused_revision
    );

    state
        .set_main_processing_paused(false)
        .expect("resume healthy live capture");
    assert_eq!(
        state.main_presented_dps_time_mode().unwrap(),
        DpsTimeMode::TimeStopAdjusted
    );
    assert_eq!(
        state
            .with_main_presented_state(|presented| presented.total_damage)
            .expect("healthy live presentation"),
        325.0
    );

    fs::remove_dir_all(config_path.parent().expect("config parent"))
        .expect("remove temporary config");
}

#[test]
fn cut_round_freezes_replay_source_and_queues_failed_persistence() {
    let config_path = temporary_config_path("cut_round_source_retry");
    let live_capture = LiveCaptureService::new(LiveCaptureResources::default());
    let mut replay = CombatState::default();
    replay.push_hit(test_hit(444.0));
    live_capture
        .restore_session(replay, CaptureQualitySource::JsonReplay)
        .expect("healthy live-capture state");
    let state = AppState::new_with_config_path(
        UiConfig::default(),
        live_capture.clone(),
        config_path.clone(),
    );
    let initial_revision = state.history_revision();

    let prepared = state
        .prepare_current_history_archive()
        .expect("healthy live-capture history snapshot")
        .expect("current replay archive");
    assert_eq!(
        prepared.summary.quality.source,
        CaptureQualitySource::JsonReplay
    );
    assert_eq!(
        state.archive_current_history_round_with(false, |_, _| Err(
            HistoryRuntimeError::Persistence
        )),
        Ok(true)
    );
    assert!(
        live_capture
            .with_state(|current| current.hits.is_empty())
            .expect("healthy live-capture state")
    );

    let pending = state
        .0
        .history
        .pending_archives
        .lock()
        .expect("pending History archives lock");
    assert_eq!(pending.len(), 1);
    assert_eq!(
        pending[0].summary.quality.source,
        CaptureQualitySource::JsonReplay,
        "retry must retain source captured at the round boundary"
    );
    assert_eq!(
        state.history_revision(),
        initial_revision,
        "a precommit failure must not advance the History read model"
    );
    drop(pending);

    let mut next = CombatState::default();
    next.push_hit(test_hit(99.0));
    live_capture
        .restore_session(next, CaptureQualitySource::Live)
        .expect("healthy live-capture state");
    assert_eq!(
        live_capture
            .with_state(|current| current.total_damage)
            .expect("healthy live-capture state"),
        99.0
    );
    let retry_template = state
        .prepare_current_history_archive()
        .expect("healthy live-capture history snapshot")
        .expect("retry queue template");
    {
        let mut pending = state
            .0
            .history
            .pending_archives
            .lock()
            .expect("pending History archives lock");
        while pending.len() < MAX_PENDING_HISTORY_ARCHIVES {
            pending.push_back(retry_template.clone());
        }
    }
    assert!(
        state
            .archive_current_history_round_with(false, |_, _| {
                panic!("a full retry queue must reject before persistence")
            })
            .is_err()
    );
    assert_eq!(
        live_capture
            .with_state(|current| current.total_damage)
            .expect("healthy live-capture state"),
        99.0,
        "full retry queue must preserve the current live round"
    );

    fs::remove_dir_all(config_path.parent().expect("config parent"))
        .expect("remove temporary config");
}

#[test]
fn committed_maintenance_warning_is_not_requeued_and_bumps_revision_once() {
    let live_capture = LiveCaptureService::new(LiveCaptureResources::default());
    let mut current = CombatState::default();
    current.push_hit(test_hit(512.0));
    live_capture
        .restore_session(current, CaptureQualitySource::PcapngReplay)
        .expect("healthy live-capture state");
    let state = AppState::new(UiConfig::default(), live_capture);
    let initial_revision = state.history_revision();
    let attempts = AtomicU64::new(0);

    assert_eq!(
        state.archive_current_history_round_with(false, |state, _| {
            attempts.fetch_add(1, Ordering::AcqRel);
            Ok(state.finish_borrowed_history_save(
                BorrowedHistorySaveOutcome::CommittedWithMaintenanceWarning(
                    nte_dps_tool::storage::history::HistoryMaintenanceWarning::RetentionPruneFailed,
                ),
            ))
        }),
        Ok(true)
    );
    assert_eq!(attempts.load(Ordering::Acquire), 1);
    assert_eq!(state.history_revision(), initial_revision + 1);
    assert!(
        state
            .0
            .history
            .pending_archives
            .lock()
            .expect("pending History archives lock")
            .is_empty(),
        "post-commit maintenance warnings must not retry the committed archive"
    );
}

#[test]
fn automatic_history_round_keeps_ineligible_state_but_manual_cut_still_works() {
    for enabled in [false, true] {
        let live_capture = LiveCaptureService::new(LiveCaptureResources::default());
        let mut current = CombatState::default();
        current.push_hit(test_hit(321.0));
        live_capture
            .restore_session(current, CaptureQualitySource::Live)
            .unwrap();
        let config = UiConfig {
            auto_round_after_idle: enabled,
            ..UiConfig::default()
        };
        let state = AppState::new(config, live_capture.clone());
        let revision = state.history_revision();
        assert_eq!(
            state.archive_current_history_round_with(true, |_, _| {
                panic!("disabled or non-running auto-round must not reach storage");
            }),
            Ok(false)
        );
        assert_eq!(state.history_revision(), revision);
        assert_eq!(
            live_capture.with_state(|state| state.total_damage).unwrap(),
            321.0
        );
        assert_eq!(
            state.archive_current_history_round_with(false, |_, archive| {
                assert_eq!(archive.summary.total_damage, 321.0);
                Ok(HistoryPersistenceOutcome::Committed)
            }),
            Ok(true)
        );
    }
}

#[test]
fn permanent_history_failure_is_reported_once_and_never_enters_the_retry_queue() {
    let live_capture = LiveCaptureService::new(LiveCaptureResources::default());
    let mut current = CombatState::default();
    current.push_hit(test_hit(700.0));
    live_capture
        .restore_session(current, CaptureQualitySource::Live)
        .expect("healthy live-capture state");
    let state = AppState::new(UiConfig::default(), live_capture);
    let attempts = AtomicU64::new(0);

    assert_eq!(
        state.archive_current_history_round_with(false, |_, _| {
            attempts.fetch_add(1, Ordering::AcqRel);
            Err(HistoryRuntimeError::PermanentPersistence)
        }),
        Err(HistoryRuntimeError::PermanentPersistence)
    );
    assert_eq!(attempts.load(Ordering::Acquire), 1);
    assert!(
        state
            .0
            .history
            .pending_archives
            .lock()
            .expect("pending History archives lock")
            .is_empty(),
        "permanent validation failures must not create an infinite retry loop"
    );
}

#[test]
fn poisoned_history_transaction_does_not_run_the_action() {
    let state = AppState::default();
    let initial_revision = state.history_revision();
    let poison_state = state.clone();
    let _ = std::thread::spawn(move || {
        let _guard = poison_state
            .0
            .history
            .transaction
            .lock()
            .expect("history transaction lock");
        panic!("poison History transaction");
    })
    .join();

    let action_ran = AtomicBool::new(false);
    let _ = state.with_history_transaction(|| {
        action_ran.store(true, Ordering::Release);
        state.bump_history_revision();
    });

    assert!(
        !action_ran.load(Ordering::Acquire),
        "a poisoned History transaction must fail before its action runs"
    );
    assert_eq!(state.history_revision(), initial_revision);
}

#[test]
fn poisoned_history_transaction_keeps_the_live_round_before_persistence() {
    let live_capture = LiveCaptureService::new(LiveCaptureResources::default());
    let mut current = CombatState::default();
    current.push_hit(test_hit(123.0));
    live_capture
        .restore_session(current, CaptureQualitySource::Live)
        .expect("healthy live-capture state");
    let state = AppState::new(UiConfig::default(), live_capture.clone());
    let initial_revision = state.history_revision();
    let poison_state = state.clone();
    let _ = std::thread::spawn(move || {
        let _guard = poison_state
            .0
            .history
            .transaction
            .lock()
            .expect("history transaction lock");
        panic!("poison History transaction before round cut");
    })
    .join();

    assert!(state.archive_current_history_round().is_err());
    assert_eq!(
        live_capture
            .with_state(|current| current.total_damage)
            .expect("healthy live-capture state"),
        123.0,
        "History runtime failure must be detected before the live round is cut"
    );
    assert_eq!(state.history_revision(), initial_revision);
}

#[test]
fn poisoned_history_maintenance_reports_unavailable() {
    let state = AppState::default();
    let poison_state = state.clone();
    let _ = std::thread::spawn(move || {
        let _guard = poison_state
            .0
            .history
            .transaction
            .lock()
            .expect("history transaction lock");
        panic!("poison History maintenance transaction");
    })
    .join();

    assert_eq!(
        state.maintain_history_rounds(),
        Err(HistoryRuntimeError::Unavailable)
    );
}

#[test]
fn history_retry_releases_transaction_and_requeues_unavailable_tail_in_order() {
    fn archive(damage: f64, source: CaptureQualitySource) -> PreparedHistoryArchive {
        let mut state = CombatState::default();
        state.push_hit(test_hit(damage));
        prepare_history_archive(&state, source, DpsTimeBasis::SubtractTimeStop, false)
            .expect("test History archive")
    }

    let state = AppState::default();
    {
        let mut pending = state
            .0
            .history
            .pending_archives
            .lock()
            .expect("pending History archives lock");
        pending.push_back(archive(1.0, CaptureQualitySource::Live));
        pending.push_back(archive(2.0, CaptureQualitySource::JsonReplay));
        pending.push_back(archive(3.0, CaptureQualitySource::PcapngReplay));
    }
    let mut attempted = Vec::new();

    let result = state.retry_pending_history_archives_with(|state, archive| {
        assert!(
            state.0.history.transaction.try_lock().is_ok(),
            "retry I/O must acquire a fresh per-item History transaction"
        );
        attempted.push(archive.summary.total_damage);
        if archive.summary.total_damage == 2.0 {
            Err(HistoryRuntimeError::Unavailable)
        } else {
            Ok(HistoryPersistenceOutcome::Committed)
        }
    });

    assert_eq!(result, Err(HistoryRuntimeError::Unavailable));
    assert_eq!(attempted, vec![1.0, 2.0]);
    let pending = state
        .0
        .history
        .pending_archives
        .lock()
        .expect("pending History archives lock");
    assert_eq!(
        pending
            .iter()
            .map(|archive| (archive.summary.total_damage, archive.summary.quality.source))
            .collect::<Vec<_>>(),
        vec![
            (2.0, CaptureQualitySource::JsonReplay),
            (3.0, CaptureQualitySource::PcapngReplay),
        ]
    );
}

#[test]
fn history_retry_consumes_committed_warning_before_requeueing_the_unavailable_tail() {
    fn archive(damage: f64, source: CaptureQualitySource) -> PreparedHistoryArchive {
        let mut state = CombatState::default();
        state.push_hit(test_hit(damage));
        prepare_history_archive(&state, source, DpsTimeBasis::SubtractTimeStop, false)
            .expect("test History archive")
    }

    let state = AppState::default();
    {
        let mut pending = state
            .0
            .history
            .pending_archives
            .lock()
            .expect("pending History archives lock");
        pending.push_back(archive(1.0, CaptureQualitySource::JsonReplay));
        pending.push_back(archive(2.0, CaptureQualitySource::PcapngReplay));
    }
    let initial_revision = state.history_revision();
    let mut attempted = Vec::new();

    let result = state.retry_pending_history_archives_with(|state, archive| {
        attempted.push((archive.summary.total_damage, archive.summary.quality.source));
        if archive.summary.total_damage == 1.0 {
            Ok(state.finish_borrowed_history_save(
                BorrowedHistorySaveOutcome::CommittedWithMaintenanceWarning(
                    nte_dps_tool::storage::history::HistoryMaintenanceWarning::RetentionPruneFailed,
                ),
            ))
        } else {
            Err(HistoryRuntimeError::Unavailable)
        }
    });

    assert_eq!(result, Err(HistoryRuntimeError::Unavailable));
    assert_eq!(
        attempted,
        vec![
            (1.0, CaptureQualitySource::JsonReplay),
            (2.0, CaptureQualitySource::PcapngReplay),
        ]
    );
    assert_eq!(state.history_revision(), initial_revision + 1);
    let pending = state
        .0
        .history
        .pending_archives
        .lock()
        .expect("pending History archives lock");
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].summary.total_damage, 2.0);
    assert_eq!(
        pending[0].summary.quality.source,
        CaptureQualitySource::PcapngReplay
    );
}

#[test]
fn history_retry_drops_permanent_failure_and_continues_with_the_remaining_queue() {
    fn archive(damage: f64) -> PreparedHistoryArchive {
        let mut state = CombatState::default();
        state.push_hit(test_hit(damage));
        prepare_history_archive(
            &state,
            CaptureQualitySource::Live,
            DpsTimeBasis::SubtractTimeStop,
            false,
        )
        .expect("test History archive")
    }

    let state = AppState::default();
    {
        let mut pending = state
            .0
            .history
            .pending_archives
            .lock()
            .expect("pending History archives lock");
        pending.push_back(archive(1.0));
        pending.push_back(archive(2.0));
    }
    let mut attempted = Vec::new();

    assert_eq!(
        state.retry_pending_history_archives_with(|_, archive| {
            attempted.push(archive.summary.total_damage);
            if archive.summary.total_damage == 1.0 {
                Err(HistoryRuntimeError::PermanentPersistence)
            } else {
                Ok(HistoryPersistenceOutcome::Committed)
            }
        }),
        Ok(())
    );
    assert_eq!(attempted, vec![1.0, 2.0]);
    assert!(
        state
            .0
            .history
            .pending_archives
            .lock()
            .expect("pending History archives lock")
            .is_empty()
    );
}

#[test]
fn poisoned_history_archive_transaction_keeps_the_live_round() {
    let live_capture = LiveCaptureService::new(LiveCaptureResources::default());
    let mut current = CombatState::default();
    current.push_hit(test_hit(321.0));
    live_capture
        .restore_session(current, CaptureQualitySource::Live)
        .expect("healthy live-capture state");
    let state = AppState::new(UiConfig::default(), live_capture.clone());
    let initial_revision = state.history_revision();
    let poison_state = state.clone();
    let _ = std::thread::spawn(move || {
        let _guard = poison_state
            .0
            .history
            .archive_transaction
            .lock()
            .expect("history archive transaction lock");
        panic!("poison History archive transaction");
    })
    .join();

    let persist_ran = AtomicBool::new(false);
    assert!(
        state
            .archive_current_history_round_with(false, |_, _| {
                persist_ran.store(true, Ordering::Release);
                Ok(HistoryPersistenceOutcome::Committed)
            })
            .is_err()
    );
    assert_eq!(state.history_revision(), initial_revision);
    assert!(!persist_ran.load(Ordering::Acquire));
    assert_eq!(
        live_capture
            .with_state(|current| current.total_damage)
            .expect("healthy live-capture state"),
        321.0,
        "the live round must not be cut when archive serialization is unavailable"
    );
}

#[test]
fn poisoned_pending_history_queue_keeps_the_live_round() {
    let live_capture = LiveCaptureService::new(LiveCaptureResources::default());
    let mut current = CombatState::default();
    current.push_hit(test_hit(654.0));
    live_capture
        .restore_session(current, CaptureQualitySource::Live)
        .expect("healthy live-capture state");
    let state = AppState::new(UiConfig::default(), live_capture.clone());
    let poison_state = state.clone();
    let _ = std::thread::spawn(move || {
        let _guard = poison_state
            .0
            .history
            .pending_archives
            .lock()
            .expect("pending History archives lock");
        panic!("poison pending History archives");
    })
    .join();

    let persist_ran = AtomicBool::new(false);
    let initial_revision = state.history_revision();
    assert!(
        state
            .archive_current_history_round_with(false, |_, _| {
                persist_ran.store(true, Ordering::Release);
                Ok(HistoryPersistenceOutcome::Committed)
            })
            .is_err()
    );
    assert!(!persist_ran.load(Ordering::Acquire));
    assert_eq!(state.history_revision(), initial_revision);
    assert_eq!(
        live_capture
            .with_state(|current| current.total_damage)
            .expect("healthy live-capture state"),
        654.0,
        "the live round must not be cut when retry capacity cannot be checked"
    );
}

#[test]
fn poisoned_history_round_cache_is_discarded_before_rebuild() {
    let state = AppState::default();
    let poison_state = state.clone();
    let _ = std::thread::spawn(move || {
        let mut cache = poison_state
            .0
            .history
            .round_cache
            .lock()
            .expect("History round cache lock");
        cache.revision = Some(poison_state.history_revision());
        cache.index = Arc::new(vec![HistoryRoundIndex::for_test(
            "poisoned-cache-entry",
            false,
        )]);
        panic!("poison History round cache");
    })
    .join();

    let index = state.main_round_index();

    assert!(
        index
            .iter()
            .all(|record| record.id != "poisoned-cache-entry"),
        "a partially mutated cache must never be projected"
    );
    assert!(
        !state.0.history.round_cache.is_poisoned(),
        "a rebuilt cache must clear the poison flag"
    );
}

#[test]
fn poisoned_history_undo_slot_rejects_mutation() {
    let state = AppState::default();
    let initial_revision = state.history_revision();
    let (directory, tombstone) = temporary_history_tombstone("poisoned-slot");
    let poison_state = state.clone();
    let _ = std::thread::spawn(move || {
        let _guard = poison_state
            .0
            .history
            .undo
            .lock()
            .expect("History undo lock");
        panic!("poison History undo slot");
    })
    .join();

    let result = state.remember_deleted_history_tombstone(tombstone.clone());

    assert!(result.is_err());
    assert_eq!(state.history_revision(), initial_revision);
    discard_tombstoned_record(&tombstone).expect("discard unused tombstone fixture");
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn history_persistence_error_display_is_stable_and_redacted() {
    let error = HistoryRuntimeError::Persistence;

    assert_eq!(error.to_string(), "History persistence did not finish.");
    assert!(!error.to_string().contains("private-storage-detail"));
}

#[test]
fn reset_undo_restores_session_and_rejects_a_wrong_token_without_consuming_it() {
    let config_path = temporary_config_path("session_reset_undo");
    let live_capture = LiveCaptureService::new(LiveCaptureResources::default());
    let mut previous = CombatState::default();
    previous.push_hit(test_hit(222.0));
    live_capture
        .restore_session(previous, CaptureQualitySource::Live)
        .expect("healthy live-capture state");
    let state =
        AppState::new_with_config_path(UiConfig::default(), live_capture, config_path.clone());

    state
        .set_main_processing_paused(true)
        .expect("pause healthy live capture");
    let token = state
        .reset_session_with_undo()
        .expect("healthy live-capture reset")
        .expect("reset undo token");
    assert!(
        !state
            .session_has_data()
            .expect("healthy live-capture state")
    );
    assert!(!state.main_processing_paused());
    assert_eq!(
        state.undo_session_reset("wrong-token"),
        Err(SessionUndoError::Missing)
    );
    state
        .undo_session_reset(&token)
        .expect("restore reset session");
    assert_eq!(
        state
            .with_main_presented_state(|presented| presented.total_damage)
            .expect("healthy restored presentation"),
        222.0
    );

    fs::remove_dir_all(config_path.parent().expect("config parent"))
        .expect("remove temporary config");
}

#[test]
fn failed_session_undo_restore_keeps_the_entry_for_retry() {
    let state = AppState::default();
    let entry = SessionUndoEntry {
        token: "session-undo-fixture".to_owned(),
        state: CombatState::default(),
        quality_source: CaptureQualitySource::JsonReplay,
        expires_at: Instant::now() + SESSION_UNDO_WINDOW,
    };

    *state
        .0
        .session_undo
        .entry
        .lock()
        .expect("session undo lock") = Some(entry.clone());
    let result = state.restore_session_undo_entry(&entry, |_, _| {
        Err(CoreError::new(
            CoreErrorCode::CaptureStateUnavailable,
            "private poison detail",
        ))
    });

    assert_eq!(result, Err(SessionUndoError::StateUnavailable));
    let undo = state
        .0
        .session_undo
        .entry
        .lock()
        .expect("session undo lock");
    let restored = undo.as_ref().expect("failed undo entry remains available");
    assert_eq!(restored.token, "session-undo-fixture");
    assert_eq!(restored.quality_source, CaptureQualitySource::JsonReplay);
}

#[test]
fn onboarding_progress_and_completion_persist() {
    let config_path = temporary_config_path("onboarding_progress");
    let state = AppState::new_with_config_path(
        UiConfig::default(),
        LiveCaptureService::new(LiveCaptureResources::default()),
        config_path.clone(),
    );

    state
        .set_onboarding_progress(2, false)
        .expect("save onboarding step");
    assert_eq!(state.onboarding_step(), 2);
    state
        .finish_onboarding(HudPreset::Detailed)
        .expect("finish onboarding");

    let saved: UiConfig =
        serde_json::from_str(&fs::read_to_string(&config_path).expect("saved UI config"))
            .expect("valid saved UI config");
    assert!(saved.onboarding_done);
    assert!(saved.hud.show_mini_timeline);
    assert_eq!(state.onboarding_step(), 3);

    fs::remove_dir_all(config_path.parent().expect("config parent"))
        .expect("remove temporary config");
}

#[test]
fn replay_import_reservation_blocks_live_capture_until_released() {
    let state = AppState::default();
    let reservation = state
        .begin_replay_import(false)
        .expect("reserve replay import");

    let duplicate_error = match state.begin_replay_import(false) {
        Ok(_) => panic!("second replay reservation must be rejected"),
        Err(error) => error,
    };
    assert!(matches!(
        duplicate_error,
        ReplayImportError::Capture(error)
            if error.code == CoreErrorCode::CaptureAlreadyRunning
    ));
    assert_eq!(
        state
            .request_capture_start(false)
            .expect_err("capture start must respect replay reservation")
            .code,
        CoreErrorCode::CaptureAlreadyRunning
    );

    drop(reservation);
    assert!(state.begin_replay_import(false).is_ok());
}

#[test]
fn invalid_json_replay_releases_reservation_without_replacing_session() {
    let live_capture = LiveCaptureService::new(LiveCaptureResources::default());
    let mut previous = CombatState::default();
    previous.push_hit(test_hit(432.0));
    live_capture
        .restore_session(previous, CaptureQualitySource::PcapngReplay)
        .expect("install existing session");
    let state = AppState::new(UiConfig::default(), live_capture);
    let initial_revision = state.0.live_capture.revision();
    let initial_packet_revision = state
        .0
        .live_capture
        .with_packet_state(|revision, _, _| revision)
        .expect("healthy packet projection");
    let initial_source = state
        .0
        .live_capture
        .quality_source()
        .expect("healthy capture source");
    let path = temporary_config_path("invalid_json_replay_preflight").with_extension("json");
    fs::write(&path, b"{").expect("write malformed JSON replay");

    let error = state
        .begin_replay_import(true)
        .expect("reserve replacing replay")
        .start(CaptureReplayKind::Json, path.clone(), true)
        .expect_err("malformed replay must fail synchronously");

    fs::remove_file(path).expect("remove malformed replay fixture");
    assert!(matches!(
        error,
        ReplayImportError::JsonImport(CaptureImportError::InvalidFormat)
    ));
    assert_eq!(state.0.live_capture.revision(), initial_revision);
    assert_eq!(
        state
            .0
            .live_capture
            .with_packet_state(|revision, _, _| revision)
            .expect("healthy packet projection"),
        initial_packet_revision
    );
    assert_eq!(
        state
            .0
            .live_capture
            .quality_source()
            .expect("healthy capture source"),
        initial_source
    );
    assert_eq!(
        state
            .0
            .live_capture
            .with_state(|combat| (combat.hits.len(), combat.total_damage))
            .expect("healthy capture state"),
        (1, 432.0)
    );
    assert_eq!(
        *state
            .0
            .replay_import
            .reservation
            .lock()
            .expect("healthy reservation runtime"),
        ReplayImportReservationState::Idle
    );
}

#[test]
fn poisoned_replay_import_runtime_rejects_reservation_before_replay_work() {
    let state = AppState::default();
    let initial_revision = state.0.live_capture.revision();
    let poison_state = state.clone();
    let _ = std::thread::spawn(move || {
        let _reserved = poison_state
            .0
            .replay_import
            .reservation
            .lock()
            .expect("replay import reservation lock");
        panic!("poison replay import reservation");
    })
    .join();

    let error = match state.begin_replay_import(false) {
        Ok(_) => panic!("poisoned replay import runtime must fail closed"),
        Err(error) => error,
    };

    assert!(matches!(error, ReplayImportError::RuntimeUnavailable));
    assert_eq!(
        state
            .request_capture_start(false)
            .expect_err("poisoned replay runtime must reject live capture")
            .code,
        CoreErrorCode::CaptureStateUnavailable
    );
    assert_eq!(state.0.live_capture.revision(), initial_revision);
}

#[test]
fn replay_import_reservation_lock_is_released_before_expensive_work() {
    let state = AppState::default();
    let reservation = state
        .begin_replay_import(false)
        .expect("reserve replay import");

    reservation
        .finish_with(|operation_state| {
            let reservation = operation_state
                .0
                .replay_import
                .reservation
                .try_lock()
                .expect("reservation lock must not span replay I/O or capture wait");
            assert_eq!(*reservation, ReplayImportReservationState::ReplayImport);
            Ok(())
        })
        .expect("finish replay reservation");

    assert_eq!(
        *state
            .0
            .replay_import
            .reservation
            .lock()
            .expect("replay import reservation lock"),
        ReplayImportReservationState::Idle
    );
}

#[test]
fn poisoned_replay_import_runtime_rejects_start_without_consuming_reservation() {
    let state = AppState::default();
    let reservation = state
        .begin_replay_import(false)
        .expect("reserve replay import");
    let poison_state = state.clone();
    let _ = std::thread::spawn(move || {
        let _reserved = poison_state
            .0
            .replay_import
            .reservation
            .lock()
            .expect("replay import reservation lock");
        panic!("poison replay import reservation after dialog reservation");
    })
    .join();
    let initial_revision = state.0.live_capture.revision();

    let error = reservation
        .start(
            CaptureReplayKind::Json,
            PathBuf::from("missing-poisoned-replay.json"),
            false,
        )
        .expect_err("poisoned replay import runtime must reject start");

    assert!(matches!(error, ReplayImportError::RuntimeUnavailable));
    assert_eq!(state.0.live_capture.revision(), initial_revision);
    let reserved = state
        .0
        .replay_import
        .reservation
        .lock()
        .expect_err("runtime remains poisoned")
        .into_inner();
    assert_eq!(
        *reserved,
        ReplayImportReservationState::ReplayImport,
        "Drop must not blindly mutate poisoned state"
    );
}

#[test]
fn poisoned_session_undo_runtime_rejects_reset_without_sequence_or_capture_change() {
    let live_capture = LiveCaptureService::new(LiveCaptureResources::default());
    let mut previous = CombatState::default();
    previous.push_hit(test_hit(444.0));
    live_capture
        .restore_session(previous, CaptureQualitySource::Live)
        .expect("healthy live-capture state");
    let state = AppState::new(UiConfig::default(), live_capture);
    let initial_sequence = state.0.session_undo.sequence.load(Ordering::Acquire);
    let initial_revision = state.0.live_capture.revision();
    let poison_state = state.clone();
    let _ = std::thread::spawn(move || {
        let _undo = poison_state
            .0
            .session_undo
            .entry
            .lock()
            .expect("session undo lock");
        panic!("poison session undo runtime");
    })
    .join();

    let error = state
        .reset_session_with_undo_action()
        .expect_err("poisoned session undo runtime must reject reset");

    assert_eq!(error, SessionUndoError::RuntimeUnavailable);
    assert_eq!(
        state.0.session_undo.sequence.load(Ordering::Acquire),
        initial_sequence
    );
    assert_eq!(state.0.live_capture.revision(), initial_revision);
    assert_eq!(
        state
            .0
            .live_capture
            .with_state(|current| current.total_damage)
            .expect("healthy live-capture state"),
        444.0
    );
}

#[test]
fn poisoned_session_undo_runtime_rejects_restore_without_capture_change() {
    let state = AppState::default();
    let entry = SessionUndoEntry {
        token: "poisoned-session-undo".to_owned(),
        state: {
            let mut previous = CombatState::default();
            previous.push_hit(test_hit(333.0));
            previous
        },
        quality_source: CaptureQualitySource::JsonReplay,
        expires_at: Instant::now() + SESSION_UNDO_WINDOW,
    };
    *state
        .0
        .session_undo
        .entry
        .lock()
        .expect("session undo lock") = Some(entry);
    let poison_state = state.clone();
    let _ = std::thread::spawn(move || {
        let _undo = poison_state
            .0
            .session_undo
            .entry
            .lock()
            .expect("session undo lock");
        panic!("poison session undo runtime before restore");
    })
    .join();
    let initial_revision = state.0.live_capture.revision();

    assert_eq!(
        state.undo_session_reset("poisoned-session-undo"),
        Err(SessionUndoError::RuntimeUnavailable)
    );
    assert_eq!(state.0.live_capture.revision(), initial_revision);
    assert!(
        !state
            .session_has_data()
            .expect("healthy live-capture state"),
        "poisoned undo state must not be restored"
    );
}

#[test]
fn poisoned_session_undo_runtime_rejects_clear_without_capture_change() {
    let live_capture = LiveCaptureService::new(LiveCaptureResources::default());
    let mut previous = CombatState::default();
    previous.push_hit(test_hit(555.0));
    live_capture
        .restore_session(previous, CaptureQualitySource::Live)
        .expect("healthy live-capture state");
    let state = AppState::new(UiConfig::default(), live_capture);
    let initial_revision = state.0.live_capture.revision();
    let poison_state = state.clone();
    let _ = std::thread::spawn(move || {
        let _undo = poison_state
            .0
            .session_undo
            .entry
            .lock()
            .expect("session undo lock");
        panic!("poison session undo runtime before clear");
    })
    .join();

    assert_eq!(
        state.clear_session_action(),
        Err(SessionUndoError::RuntimeUnavailable)
    );
    assert_eq!(state.0.live_capture.revision(), initial_revision);
    assert_eq!(
        state
            .0
            .live_capture
            .with_state(|current| current.total_damage)
            .expect("healthy live-capture state"),
        555.0
    );
}

#[test]
fn capture_file_refresh_advances_settings_generation() {
    let state = AppState::default();
    let generation = state.settings_revision();

    state.refresh_capture_file_stats();

    assert!(state.settings_revision() > generation);
}

#[test]
fn device_catalog_revision_effects_are_change_and_availability_aware() {
    let state = AppState::default();
    let candidate = vec![CaptureDeviceSnapshot {
        id: "revision-fixture-device".to_owned(),
        label: "Revision fixture".to_owned(),
    }];
    let settings_before = state.settings_revision();
    let main_before = state.0.presentation.main_revision.load(Ordering::Acquire);

    state
        .refresh_capture_devices_with(|| Ok(candidate.clone()))
        .expect("replace device catalog");
    assert_eq!(state.settings_revision(), settings_before + 1);
    assert_eq!(
        state.0.presentation.main_revision.load(Ordering::Acquire),
        main_before + 1
    );

    let settings_after_replace = state.settings_revision();
    let main_after_replace = state.0.presentation.main_revision.load(Ordering::Acquire);
    state
        .refresh_capture_devices_with(|| Ok(candidate.clone()))
        .expect("identical device catalog");
    assert_eq!(state.settings_revision(), settings_after_replace);
    assert_eq!(
        state.0.presentation.main_revision.load(Ordering::Acquire),
        main_after_replace
    );

    assert!(
        state
            .refresh_capture_devices_with(|| {
                Err(CoreError::new(
                    CoreErrorCode::SystemProbeFailed,
                    "private adapter detail",
                ))
            })
            .is_err()
    );
    assert_eq!(state.settings_revision(), settings_after_replace + 1);
    assert_eq!(
        state.0.presentation.main_revision.load(Ordering::Acquire),
        main_after_replace + 1
    );
    let degraded = state.0.settings.device_catalog_snapshot(|| {});
    assert!(!degraded.available);
    assert_eq!(degraded.devices, candidate);

    let settings_after_failure = state.settings_revision();
    let main_after_failure = state.0.presentation.main_revision.load(Ordering::Acquire);
    assert!(
        state
            .refresh_capture_devices_with(|| {
                Err(CoreError::new(
                    CoreErrorCode::SystemProbeFailed,
                    "another private adapter detail",
                ))
            })
            .is_err()
    );
    assert_eq!(state.settings_revision(), settings_after_failure);
    assert_eq!(
        state.0.presentation.main_revision.load(Ordering::Acquire),
        main_after_failure
    );
}

#[test]
fn encrypted_ini_session_orders_save_and_clear_generations() {
    let config_path = temporary_config_path("encrypted_ini_session");
    let ini_path = config_path.with_file_name("Engine.ini");
    fs::write(&ini_path, "Value=1\n").expect("write INI fixture");
    let state = AppState::default();

    let opened = state
        .open_encrypted_ini(ini_path.clone())
        .expect("open INI fixture");
    assert_eq!(opened.generation, 1);
    assert_eq!(opened.plaintext, "Value=1");
    assert!(matches!(
        state.save_encrypted_ini(0, "Value=2".to_owned(), EncryptedIniKey::Global,),
        Err(EncryptedIniServiceError::StaleGeneration)
    ));

    let (saved, outcome) = state
        .save_encrypted_ini(
            opened.generation,
            "Value=2".to_owned(),
            EncryptedIniKey::Global,
        )
        .expect("save INI fixture");
    assert_eq!(outcome, EncryptedIniSaveOutcome::Saved);
    assert_eq!(saved.generation, 2);
    assert_eq!(saved.plaintext, "Value=2");
    assert_eq!(
        state
            .clear_encrypted_ini()
            .expect("clear encrypted INI")
            .generation,
        3
    );

    fs::remove_dir_all(config_path.parent().expect("fixture parent")).expect("remove INI fixture");
}

#[test]
fn abyss_team_mutations_keep_each_prediction_line_explicit() {
    let state = AppState::default();
    let team = TeamDps {
        dps: 12_345.0,
        members: Vec::new(),
    };
    let export = TeamDpsExport {
        version: nte_dps_tool::engine::model::TEAM_DPS_EXPORT_VERSION,
        single: Some(team.clone()),
        upper: None,
        lower: None,
    };

    let initial_revision = state.settings_revision();
    assert!(
        state
            .import_abyss_team(export.clone(), true)
            .expect("import upper team")
    );
    assert!(state.settings_revision() > initial_revision);
    let imported_revision = state.settings_revision();
    assert!(
        state
            .import_abyss_team(export, true)
            .expect("repeat upper team")
    );
    assert_eq!(state.settings_revision(), imported_revision);
    let (upper, lower) = state.imported_abyss_teams().expect("imported teams");
    assert_eq!(upper.as_ref().map(|team| team.dps), Some(12_345.0));
    assert!(lower.is_none());

    assert!(state.swap_abyss_teams().expect("swap imported teams"));
    let (upper, lower) = state.imported_abyss_teams().expect("swapped teams");
    assert!(upper.is_none());
    assert_eq!(lower.as_ref().map(|team| team.dps), Some(12_345.0));

    assert!(state.clear_abyss_team(false).expect("clear lower team"));
    let (upper, lower) = state.imported_abyss_teams().expect("cleared teams");
    assert!(upper.is_none());
    assert!(lower.is_none());
}

#[test]
fn poisoned_team_state_is_projected_unavailable_once_without_partial_data() {
    let state = AppState::default();
    let export = TeamDpsExport {
        version: nte_dps_tool::engine::model::TEAM_DPS_EXPORT_VERSION,
        single: Some(TeamDps {
            dps: 42.0,
            members: Vec::new(),
        }),
        upper: None,
        lower: None,
    };
    state.import_team_data(export).expect("seed imported teams");
    state.0.team_import.poison_for_test();
    let revision_before_recovery = state.settings_revision();

    let snapshot = state.settings_snapshot();

    assert!(!snapshot.team_data.available);
    assert!(!snapshot.team_data.upper_imported);
    assert!(!snapshot.team_data.lower_imported);
    assert_eq!(state.settings_revision(), revision_before_recovery + 1);
    assert!(state.imported_abyss_teams().is_err());
    assert_eq!(state.settings_revision(), revision_before_recovery + 1);
}

#[test]
fn finishing_replaced_stream_keeps_current_registration() {
    let state = AppState::default();
    let previous = state
        .reserve_stream("hud", "technical")
        .expect("reserve previous stream");
    assert!(
        state
            .activate_stream(&previous)
            .expect("activate previous stream")
    );
    let current = state
        .reserve_stream("hud", "technical")
        .expect("reserve current stream");
    assert!(
        state
            .activate_stream(&current)
            .expect("activate current stream")
    );

    assert!(!state.finish_stream(&previous).expect("finish stale stream"));
    assert!(
        state
            .stop_stream("hud", "technical")
            .expect("stop current stream")
    );

    assert!(current.is_cancelled());
}

#[test]
fn destroying_owner_window_stops_only_its_streams_and_clears_registry_entries() {
    let state = AppState::default();
    let hud_first = state
        .reserve_stream("hud", "hud:first")
        .expect("reserve HUD stream");
    let hud_second = state
        .reserve_stream("hud", "hud:second")
        .expect("reserve HUD stream");
    let console = state
        .reserve_stream("console", "console:first")
        .expect("reserve Console stream");
    assert!(
        state
            .activate_stream(&hud_first)
            .expect("activate HUD stream")
    );
    assert!(
        state
            .activate_stream(&hud_second)
            .expect("activate HUD stream")
    );
    assert!(
        state
            .activate_stream(&console)
            .expect("activate Console stream")
    );
    assert_eq!(state.stream_registry_len().expect("registry length"), 3);

    assert_eq!(
        state
            .stop_streams_for_window("hud")
            .expect("stop HUD streams"),
        2
    );
    assert!(hud_first.is_cancelled());
    assert!(hud_second.is_cancelled());
    assert!(!console.is_cancelled());
    assert_eq!(
        state.stop_streams_for_window("hud").expect("repeat stop"),
        0
    );
    assert_eq!(state.stream_registry_len().expect("registry length"), 1);

    assert!(
        state
            .stop_stream("console", "console:first")
            .expect("stop Console stream")
    );
    assert!(console.is_cancelled());
    assert_eq!(state.stream_registry_len().expect("registry length"), 0);
}

#[test]
fn editor_snapshot_uses_rust_preview_and_passthrough_snapshot_is_empty() {
    let state = AppState::default();

    assert_eq!(
        state
            .snapshot()
            .expect("healthy live-capture snapshot")
            .hud
            .data_state,
        HudDataState::Preview
    );

    state.set_passthrough(true);

    assert_eq!(
        state
            .snapshot()
            .expect("healthy live-capture snapshot")
            .hud
            .data_state,
        HudDataState::Empty
    );
}

#[test]
fn initial_window_and_hud_projection_follow_loaded_config() {
    let mut config = UiConfig {
        always_on_top: false,
        passthrough_hotkey: HotkeyBinding::new(false, false, false, config::HotkeyKey::F8),
        ..UiConfig::default()
    };
    config.hud.width = 512;
    config.hud.show_total_damage = false;

    let state = AppState::new(
        config,
        LiveCaptureService::new(LiveCaptureResources::default()),
    );
    let snapshot = state.snapshot().expect("healthy live-capture snapshot");

    assert!(!state.always_on_top());
    assert_eq!(
        state.passthrough_hotkey(),
        HotkeyBinding::new(false, false, false, config::HotkeyKey::F8)
    );
    assert!(!state.passthrough_hotkey_ready());
    assert_eq!(state.hud_width(), 512);
    assert_eq!(
        state.hud_initial_height(),
        (HUD_BASE_INITIAL_HEIGHT
            + HUD_SUMMARY_HEIGHT
            + HUD_CHARACTERS_HEIGHT
            + HUD_EDITOR_MODULE_HEADER_HEIGHT * 2)
            .max(HUD_EDITOR_MIN_HEIGHT)
    );
    assert_eq!(snapshot.hud.config.width, 512);
    assert!(!snapshot.hud.config.show_total_damage);
}

#[test]
fn passthrough_hotkey_readiness_is_runtime_only() {
    let state = AppState::default();
    let initial_revision = state.stream_revision();

    state.set_passthrough_hotkey_ready(true);

    assert!(state.passthrough_hotkey_ready());
    assert_eq!(state.stream_revision(), initial_revision);
}

#[test]
fn poisoned_paused_presentation_discards_partial_snapshot_once() {
    let state = AppState::default();
    state
        .set_main_processing_paused(true)
        .expect("pause healthy presentation");
    let initial_stream_revision = state.stream_revision();
    let initial_main_revision = state.0.presentation.main_revision.load(Ordering::Acquire);
    let poison_state = state.clone();
    let _ = std::thread::spawn(move || {
        let mut paused = poison_state
            .0
            .presentation
            .mode
            .lock()
            .expect("presentation mode lock");
        let mut partial = CombatState::default();
        partial.push_hit(test_hit(987_654.0));
        paused.paused.as_mut().expect("paused snapshot").state = Arc::new(partial);
        paused.selected_round = Some(SelectedRoundPresentation {
            dps_time_mode: DpsTimeMode::RealTime,
            record_id: "private-partial-round".to_owned(),
            history_revision: 0,
            state: Arc::new(CombatState::default()),
        });
        panic!("poison paused presentation after a partial replacement");
    })
    .join();

    assert_eq!(
        state
            .with_main_presented_state(|current| current.total_damage)
            .expect("recover presentation to live"),
        0.0
    );
    assert!(!state.main_processing_paused());
    assert!(state.main_selected_round_id().is_none());
    assert!(!state.0.presentation.mode.is_poisoned());
    assert_ne!(state.stream_revision(), initial_stream_revision);
    assert_eq!(
        state.0.presentation.main_revision.load(Ordering::Acquire),
        initial_main_revision + 1
    );
    let recovered_stream_revision = state.stream_revision();
    let recovered_main_revision = state.0.presentation.main_revision.load(Ordering::Acquire);
    assert_eq!(
        state
            .with_main_presented_state(|current| current.total_damage)
            .expect("repeat healthy live projection"),
        0.0
    );
    assert_eq!(state.stream_revision(), recovered_stream_revision);
    assert_eq!(
        state.0.presentation.main_revision.load(Ordering::Acquire),
        recovered_main_revision
    );
}

#[test]
fn mode_noops_do_not_advance_technical_or_main_revisions() {
    let state = AppState::default();
    let initial_stream_revision = state.stream_revision();
    let initial_main_revision = state.0.presentation.main_revision.load(Ordering::Acquire);

    assert!(
        !state
            .set_main_processing_paused(false)
            .expect("repeat live mode")
    );
    assert!(
        !state
            .set_main_selected_round_id(None)
            .expect("repeat live round")
    );
    state.return_main_presentation_to_live();

    assert_eq!(state.stream_revision(), initial_stream_revision);
    assert_eq!(
        state.0.presentation.main_revision.load(Ordering::Acquire),
        initial_main_revision
    );
}

#[test]
fn selected_abyss_effects_are_exact_and_noop_aware() {
    let state = AppState::default();
    let initial_stream_revision = state.stream_revision();
    let initial_main_revision = state.0.presentation.main_revision.load(Ordering::Acquire);

    assert!(
        state
            .set_main_selected_abyss_half(Some(AbyssHalf::Second))
            .expect("select second abyss half")
    );

    assert_ne!(state.stream_revision(), initial_stream_revision);
    assert_eq!(
        state.0.presentation.main_revision.load(Ordering::Acquire),
        initial_main_revision + 1
    );
    let changed_stream_revision = state.stream_revision();
    let changed_main_revision = state.0.presentation.main_revision.load(Ordering::Acquire);

    assert!(
        !state
            .set_main_selected_abyss_half(Some(AbyssHalf::Second))
            .expect("repeat second abyss half")
    );

    assert_eq!(state.stream_revision(), changed_stream_revision);
    assert_eq!(
        state.0.presentation.main_revision.load(Ordering::Acquire),
        changed_main_revision
    );
}

#[test]
fn history_selection_ignores_incoming_hits_then_returns_live_on_outgoing_hit() {
    fn replay_path(tag: &str, direction: &str) -> PathBuf {
        let path = temporary_config_path(tag).with_extension("json");
        let document = serde_json::json!({
            "version": 1,
            "hits": [{
                "timestamp_unix": 1.0,
                "char_id": 7,
                "char_name": "Fixture",
                "damage": 25.0,
                "direction": direction
            }],
            "packets": []
        });
        fs::write(
            &path,
            serde_json::to_vec(&document).expect("serialize replay fixture"),
        )
        .expect("write replay fixture");
        path
    }

    fn run_replay(state: &AppState, path: PathBuf) {
        state
            .0
            .live_capture
            .request_replay(CaptureReplayKind::Json, path, None, true, false)
            .expect("start JSON replay");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while state.live_capture_status().phase != LiveCapturePhase::Stopped {
            assert!(
                std::time::Instant::now() < deadline,
                "JSON replay did not reach its stopped barrier"
            );
            std::thread::yield_now();
        }
    }

    let state = AppState::default();
    state
        .set_main_selected_abyss_half(Some(AbyssHalf::Second))
        .expect("select second half");
    let selected_outgoing_revision = state.0.live_capture.outgoing_hit_revision();
    {
        let (mut mode, recovered) = state.0.presentation.lock_mode();
        assert!(!recovered);
        mode.selected_outgoing_revision = selected_outgoing_revision;
        mode.selected_round = Some(SelectedRoundPresentation {
            dps_time_mode: DpsTimeMode::RealTime,
            record_id: "history-round".to_owned(),
            history_revision: state.history_revision(),
            state: Arc::new(CombatState::default()),
        });
    }

    let incoming_path = replay_path("history_incoming_stays_selected", "incoming");
    run_replay(&state, incoming_path.clone());
    assert_eq!(
        state.0.live_capture.outgoing_hit_revision(),
        selected_outgoing_revision
    );
    assert_eq!(
        state.main_selected_round_id().as_deref(),
        Some("history-round")
    );
    assert_eq!(
        state.abyss_presentation_snapshot().selected,
        Some(AbyssHalf::Second)
    );

    let outgoing_path = replay_path("history_outgoing_returns_live", "outgoing");
    run_replay(&state, outgoing_path.clone());
    assert_eq!(
        state.0.live_capture.outgoing_hit_revision(),
        selected_outgoing_revision + 1
    );
    assert!(state.main_selected_round_id().is_none());
    assert_eq!(
        state.abyss_presentation_snapshot().selected,
        Some(AbyssHalf::Second),
        "switching back to live must not erase the user's half selection"
    );

    fs::remove_dir_all(incoming_path.parent().expect("incoming fixture parent"))
        .expect("remove incoming fixture");
    fs::remove_dir_all(outgoing_path.parent().expect("outgoing fixture parent"))
        .expect("remove outgoing fixture");
}

#[test]
fn detail_request_effect_is_exact_and_noop_aware() {
    let state = AppState::default();
    let initial_stream_revision = state.stream_revision();
    let initial_main_revision = state.0.presentation.main_revision.load(Ordering::Acquire);
    let request = MainDpsDetailRequest {
        character_id: Some(10),
        filter: CombatDetailFilter::Outgoing,
        skill_filter: Some("skill".to_owned()),
    };

    assert!(
        state
            .set_main_dps_detail_request(MainDpsDetailKind::Character, request.clone())
            .expect("set character detail request")
    );

    assert_eq!(state.stream_revision(), initial_stream_revision);
    assert_eq!(
        state.0.presentation.main_revision.load(Ordering::Acquire),
        initial_main_revision + 1
    );
    assert!(
        !state
            .set_main_dps_detail_request(MainDpsDetailKind::Character, request)
            .expect("repeat character detail request")
    );
    assert_eq!(
        state.0.presentation.main_revision.load(Ordering::Acquire),
        initial_main_revision + 1
    );
}

#[test]
fn poisoned_abyss_domain_resets_both_fields_once() {
    let state = AppState::default();
    let initial_stream_revision = state.stream_revision();
    let initial_main_revision = state.0.presentation.main_revision.load(Ordering::Acquire);
    let poison_state = state.clone();
    let _ = std::thread::spawn(move || {
        let mut abyss = poison_state
            .0
            .presentation
            .abyss
            .lock()
            .expect("abyss presentation lock");
        abyss.selected = Some(AbyssHalf::Second);
        abyss.observed = Some(AbyssHalf::First);
        panic!("poison abyss presentation after a partial transition");
    })
    .join();

    assert_eq!(
        state.abyss_presentation_snapshot(),
        AbyssPresentationState::default()
    );
    assert!(!state.0.presentation.abyss.is_poisoned());
    assert_ne!(state.stream_revision(), initial_stream_revision);
    assert_eq!(
        state.0.presentation.main_revision.load(Ordering::Acquire),
        initial_main_revision + 1
    );
    let recovered_stream_revision = state.stream_revision();
    let recovered_main_revision = state.0.presentation.main_revision.load(Ordering::Acquire);
    assert_eq!(
        state.abyss_presentation_snapshot(),
        AbyssPresentationState::default()
    );
    assert_eq!(state.stream_revision(), recovered_stream_revision);
    assert_eq!(
        state.0.presentation.main_revision.load(Ordering::Acquire),
        recovered_main_revision
    );
}

#[test]
fn poisoned_detail_domain_resets_both_requests_once() {
    let state = AppState::default();
    let initial_stream_revision = state.stream_revision();
    let initial_main_revision = state.0.presentation.main_revision.load(Ordering::Acquire);
    let poison_state = state.clone();
    let _ = std::thread::spawn(move || {
        let mut details = poison_state
            .0
            .presentation
            .details
            .lock()
            .expect("detail presentation lock");
        details.character.character_id = Some(7);
        details.team.skill_filter = Some("private-partial-filter".to_owned());
        panic!("poison detail presentation after a partial transition");
    })
    .join();

    assert_eq!(
        state.main_dps_detail_request(MainDpsDetailKind::Character),
        MainDpsDetailRequest::default()
    );
    assert_eq!(
        state.main_dps_detail_request(MainDpsDetailKind::Team),
        MainDpsDetailRequest::default()
    );
    assert!(!state.0.presentation.details.is_poisoned());
    assert_eq!(state.stream_revision(), initial_stream_revision);
    assert_eq!(
        state.0.presentation.main_revision.load(Ordering::Acquire),
        initial_main_revision + 1
    );
    let recovered_main_revision = state.0.presentation.main_revision.load(Ordering::Acquire);
    assert_eq!(
        state.main_dps_detail_request(MainDpsDetailKind::Character),
        MainDpsDetailRequest::default()
    );
    assert_eq!(
        state.0.presentation.main_revision.load(Ordering::Acquire),
        recovered_main_revision
    );
}

#[test]
fn poisoned_detail_mutation_reports_unavailable_after_reset() {
    let state = AppState::default();
    let poison_state = state.clone();
    let _ = std::thread::spawn(move || {
        let _details = poison_state
            .0
            .presentation
            .details
            .lock()
            .expect("detail presentation lock");
        panic!("poison detail presentation before a command mutation");
    })
    .join();

    assert!(matches!(
        state.set_main_dps_detail_request(
            MainDpsDetailKind::Character,
            MainDpsDetailRequest {
                character_id: Some(42),
                ..Default::default()
            },
        ),
        Err(PresentationError::StateUnavailable)
    ));
    assert_eq!(
        state.main_dps_detail_request(MainDpsDetailKind::Character),
        MainDpsDetailRequest::default()
    );
}

#[test]
fn poisoned_detail_cache_is_a_clean_miss_without_revision_effect() {
    let state = AppState::default();
    let initial_stream_revision = state.stream_revision();
    let initial_main_revision = state.0.presentation.main_revision.load(Ordering::Acquire);
    let poison_state = state.clone();
    let _ = std::thread::spawn(move || {
        let _cache = poison_state
            .0
            .presentation
            .cache
            .lock()
            .expect("detail cache lock");
        panic!("poison detail cache");
    })
    .join();

    assert!(
        state
            .main_dps_detail_cache_get(
                MainDpsStreamRevision {
                    capture: 0,
                    packet: 0,
                    presentation: 0,
                    history: 0,
                    main: 0,
                },
                MainDpsDetailKind::Character,
                &MainDpsDetailRequest::default(),
                0,
                1,
            )
            .is_none()
    );
    assert!(!state.0.presentation.cache.is_poisoned());
    assert_eq!(state.stream_revision(), initial_stream_revision);
    assert_eq!(
        state.0.presentation.main_revision.load(Ordering::Acquire),
        initial_main_revision
    );
}

#[test]
fn live_abyss_selection_follows_only_real_half_transitions() {
    assert_eq!(
        next_live_abyss_selection(None, None, Some(AbyssHalf::First)),
        (Some(AbyssHalf::First), Some(AbyssHalf::First))
    );
    assert_eq!(
        next_live_abyss_selection(
            Some(AbyssHalf::First),
            Some(AbyssHalf::First),
            Some(AbyssHalf::Second),
        ),
        (Some(AbyssHalf::Second), Some(AbyssHalf::Second))
    );
    assert_eq!(
        next_live_abyss_selection(
            Some(AbyssHalf::First),
            Some(AbyssHalf::Second),
            Some(AbyssHalf::Second),
        ),
        (Some(AbyssHalf::First), Some(AbyssHalf::Second))
    );
    assert_eq!(
        next_live_abyss_selection(Some(AbyssHalf::Second), Some(AbyssHalf::Second), None,),
        (None, None)
    );
}

#[test]
fn detailed_hud_initial_height_reserves_optional_modules() {
    let config = UiConfig {
        hud: HudConfig::detailed(),
        ..UiConfig::default()
    };
    let state = AppState::new(
        config,
        LiveCaptureService::new(LiveCaptureResources::default()),
    );

    assert_eq!(
        state.hud_initial_height(),
        HUD_BASE_INITIAL_HEIGHT
            + HUD_SUMMARY_HEIGHT
            + HUD_CHARACTERS_HEIGHT
            + HUD_OPTIONAL_TITLE_HEIGHT
            + HUD_OPTIONAL_STATUS_HEIGHT
            + HUD_MINI_TIMELINE_HEIGHT
            + HUD_EDITOR_MODULE_HEADER_HEIGHT * 5
    );
}

#[test]
fn hidden_core_modules_keep_the_full_editor_height() {
    let mut config = UiConfig::default();
    config.hud.set_module_visible(HudModule::Summary, false);
    config.hud.set_module_visible(HudModule::Characters, false);
    let config_path = temporary_config_path("hidden_core_module_height");
    let state = AppState::new_with_config_path(
        config,
        LiveCaptureService::new(LiveCaptureResources::default()),
        config_path.clone(),
    );

    assert_eq!(state.hud_initial_height(), HUD_EDITOR_MIN_HEIGHT);
    fs::remove_dir_all(config_path.parent().expect("config parent"))
        .expect("remove temporary config");
}

#[test]
fn stream_revision_changes_only_when_presentation_state_changes() {
    let state = AppState::default();
    let initial = state.stream_revision();

    state.set_passthrough(false);
    assert!(
        !state
            .set_always_on_top(state.always_on_top())
            .expect("same-value always-on-top")
    );
    assert_eq!(state.stream_revision(), initial);

    state.set_passthrough(true);
    assert_ne!(state.stream_revision(), initial);
}

#[test]
fn settings_revision_tracks_async_update_status_changes() {
    let state = AppState::default();
    let initial = state.settings_revision();

    state.begin_update_check().expect("begin update check");

    assert!(state.settings_revision() > initial);
    assert_eq!(state.settings_snapshot().updates.status, "checking");
}

#[test]
fn update_runtime_projects_available_download_and_prepared_states() {
    let state = AppState::default();
    let available = AvailableComponentUpdate {
        component: UpdateComponent::App,
        release_id: "release".to_owned(),
        version: "0.4.0".parse().expect("semantic version"),
        published_at: "2026-07-31T00:00:00Z".to_owned(),
        notes: "notes".to_owned(),
        artifact_url: "https://example.invalid/app.zip".to_owned(),
        artifact_size: 1_024,
        artifact_sha256: [7; 32],
    };

    state.begin_update_check().expect("begin update check");
    state
        .finish_update_check(vec![available])
        .expect("finish update check");
    let snapshot = state.settings_snapshot();
    assert_eq!(snapshot.updates.status, "available");
    assert_eq!(
        snapshot.updates.available[0].component,
        UpdateComponent::App
    );

    state
        .begin_update_download(UpdateComponent::App)
        .expect("begin update download");
    assert!(
        state
            .update_download_progress(UpdateComponent::App, 512, 1_024)
            .expect("update download progress")
    );
    let progress_revision = state.settings_revision();
    assert!(
        !state
            .update_download_progress(UpdateComponent::App, 512, 1_024)
            .expect("repeat update download progress")
    );
    assert_eq!(state.settings_revision(), progress_revision);
    let snapshot = state.settings_snapshot();
    assert_eq!(snapshot.updates.status, "downloading");
    assert_eq!(snapshot.updates.downloaded_bytes, "512");

    state
        .finish_update_download(PreparedUpdate::App {
            version: "0.4.0".parse().expect("semantic version"),
            transaction_path: PathBuf::from("transaction.json"),
            updater_path: PathBuf::from("nte-updater.exe"),
        })
        .expect("finish update download");
    let snapshot = state.settings_snapshot();
    assert_eq!(snapshot.updates.status, "ready");
    assert_eq!(
        snapshot
            .updates
            .prepared
            .as_ref()
            .map(|item| item.component),
        Some(UpdateComponent::App)
    );
    assert!(snapshot.updates.install_enabled);
}

#[test]
fn poisoned_update_runtime_fails_closed_without_revision_effect() {
    let state = AppState::default();
    let available = AvailableComponentUpdate {
        component: UpdateComponent::App,
        release_id: "release".to_owned(),
        version: "0.4.0".parse().expect("semantic version"),
        published_at: "2026-07-31T00:00:00Z".to_owned(),
        notes: "notes".to_owned(),
        artifact_url: "https://example.invalid/app.zip".to_owned(),
        artifact_size: 1_024,
        artifact_sha256: [7; 32],
    };
    state.begin_update_check().expect("begin update check");
    state
        .finish_update_check(vec![available])
        .expect("finish update check");
    state
        .begin_update_download(UpdateComponent::App)
        .expect("begin update download");
    state
        .finish_update_download(PreparedUpdate::App {
            version: "0.4.0".parse().expect("semantic version"),
            transaction_path: PathBuf::from("private-update-transaction.json"),
            updater_path: PathBuf::from("private-updater.exe"),
        })
        .expect("finish update download");
    let initial_revision = state.settings_revision();
    state.poison_update_runtime_for_test();

    assert_eq!(
        state.begin_update_check(),
        Err(UpdateActionError::RuntimeUnavailable)
    );
    assert_eq!(
        state.begin_update_download(UpdateComponent::App),
        Err(UpdateActionError::RuntimeUnavailable)
    );
    assert!(matches!(
        state.begin_update_install(),
        Err(UpdateActionError::RuntimeUnavailable)
    ));
    assert_eq!(state.settings_revision(), initial_revision);
    let snapshot = state.settings_snapshot();
    assert_eq!(snapshot.updates.status, "unavailable");
    assert_eq!(
        snapshot.updates.message_key,
        "Update operation did not finish."
    );
    assert!(snapshot.updates.available.is_empty());
    assert!(snapshot.updates.prepared.is_none());
    assert!(!snapshot.updates.install_enabled);
    let serialized =
        serde_json::to_string(&snapshot.updates).expect("serialize unavailable update state");
    assert!(!serialized.contains("private-update-transaction"));
    assert!(!serialized.contains("private-updater"));
}

#[test]
fn module_visibility_is_saved_before_the_projection_changes() {
    let config_path = temporary_config_path("module_visibility");
    let state = AppState::new_with_config_path(
        UiConfig::default(),
        LiveCaptureService::new(LiveCaptureResources::default()),
        config_path.clone(),
    );
    let initial_revision = state.stream_revision();

    assert!(
        state
            .set_hud_module_visibility(HudModule::Timeline, true)
            .expect("module visibility save")
    );
    assert!(
        state
            .snapshot()
            .expect("healthy live-capture snapshot")
            .hud
            .config
            .show_mini_timeline
    );
    assert_ne!(state.stream_revision(), initial_revision);

    let saved: UiConfig =
        serde_json::from_str(&fs::read_to_string(&config_path).expect("saved UI config"))
            .expect("valid saved UI config");
    assert!(saved.hud.show_mini_timeline);

    let revision = state.stream_revision();
    assert!(
        !state
            .set_hud_module_visibility(HudModule::Timeline, true)
            .expect("same-value module visibility")
    );
    assert_eq!(state.stream_revision(), revision);

    fs::remove_dir_all(config_path.parent().expect("config parent"))
        .expect("remove temporary config");
}

#[test]
fn settings_hud_option_is_persisted_and_projected() {
    let config_path = temporary_config_path("settings_hud_option");
    let state = AppState::new_with_config_path(
        UiConfig::default(),
        LiveCaptureService::new(LiveCaptureResources::default()),
        config_path.clone(),
    );

    assert!(
        state
            .set_hud_option(HudSettingOption::DamageTaken, true)
            .expect("HUD option save")
    );
    assert!(state.settings_snapshot().hud.show_damage_taken);

    let saved: UiConfig =
        serde_json::from_str(&fs::read_to_string(&config_path).expect("saved UI config"))
            .expect("valid saved UI config");
    assert!(saved.hud.show_damage_taken);

    fs::remove_dir_all(config_path.parent().expect("config parent"))
        .expect("remove temporary config");
}

#[test]
fn config_noop_and_density_revision_effects_are_exact() {
    let config_path = temporary_config_path("settings_density_effects");
    let state = AppState::new_with_config_path(
        UiConfig::default(),
        LiveCaptureService::new(LiveCaptureResources::default()),
        config_path.clone(),
    );
    let initial_settings = state.settings_revision();
    let initial_technical = state.0.presentation.revision.load(Ordering::Acquire);
    let initial_main = state.0.presentation.main_revision.load(Ordering::Acquire);
    let initial_history = state.history_revision();

    assert!(!state.set_density(UiDensity::Cozy).expect("density no-op"));
    assert_eq!(state.settings_revision(), initial_settings);
    assert_eq!(
        state.0.presentation.revision.load(Ordering::Acquire),
        initial_technical
    );
    assert_eq!(
        state.0.presentation.main_revision.load(Ordering::Acquire),
        initial_main
    );
    assert_eq!(state.history_revision(), initial_history);
    assert!(!config_path.exists());

    assert!(
        state
            .set_density(UiDensity::Compact)
            .expect("change density")
    );
    assert_eq!(state.settings_revision(), initial_settings + 1);
    assert_eq!(
        state.0.presentation.revision.load(Ordering::Acquire),
        initial_technical
    );
    assert_eq!(
        state.0.presentation.main_revision.load(Ordering::Acquire),
        initial_main + 1
    );
    assert_eq!(state.history_revision(), initial_history);

    let changed_settings = state.settings_revision();
    let changed_main = state.0.presentation.main_revision.load(Ordering::Acquire);
    assert!(
        !state
            .set_density(UiDensity::Compact)
            .expect("repeat density")
    );
    assert_eq!(state.settings_revision(), changed_settings);
    assert_eq!(
        state.0.presentation.main_revision.load(Ordering::Acquire),
        changed_main
    );

    fs::remove_dir_all(config_path.parent().expect("config parent"))
        .expect("remove temporary config");
}

#[test]
fn settings_snapshot_uses_one_committed_config_generation() {
    let config_path = temporary_config_path("settings_snapshot_generation");
    let state = AppState::new_with_config_path(
        UiConfig::default(),
        LiveCaptureService::new(LiveCaptureResources::default()),
        config_path.clone(),
    );
    let before = state.settings_revision();
    std::thread::scope(|scope| {
        let writer = scope.spawn(|| {
            state
                .update_ui_config_with_effects(SettingsMutationEffects::SETTINGS, |config| {
                    config.dark_mode = true;
                    config.auto_check_updates = false;
                })
                .expect("commit replacement config generation");
        });
        let check = |snapshot: SettingsSnapshot| {
            let updated = snapshot.interface.dark_mode;
            assert_eq!(
                snapshot.generation,
                (before + u64::from(updated)).to_string()
            );
            assert_eq!(snapshot.updates.auto_check, !updated);
        };
        check(state.settings_snapshot());
        writer.join().unwrap();
        let committed = state.settings_snapshot();
        assert_eq!(committed.generation, (before + 1).to_string());
        check(committed);
    });

    fs::remove_dir_all(config_path.parent().expect("config parent"))
        .expect("remove temporary config");
}

#[test]
fn settings_hud_preset_preserves_width_and_canonical_order() {
    let config_path = temporary_config_path("settings_hud_preset");
    let mut config = UiConfig::default();
    config.hud.width = 620;
    config.hud.module_order = vec![
        HudModule::Timeline,
        HudModule::Characters,
        HudModule::Status,
        HudModule::Summary,
        HudModule::Title,
    ];
    let state = AppState::new_with_config_path(
        config,
        LiveCaptureService::new(LiveCaptureResources::default()),
        config_path.clone(),
    );

    assert!(
        state
            .apply_hud_preset(HudPreset::Detailed)
            .expect("HUD preset save")
    );
    let snapshot = state.settings_snapshot();
    assert_eq!(snapshot.hud.width, 620);
    assert_eq!(
        snapshot.hud.module_order,
        [
            HudModule::Timeline,
            HudModule::Characters,
            HudModule::Status,
            HudModule::Summary,
            HudModule::Title,
        ]
    );
    assert!(snapshot.hud.show_mini_timeline);
    assert!(snapshot.hud.show_damage_taken);

    fs::remove_dir_all(config_path.parent().expect("config parent"))
        .expect("remove temporary config");
}

#[test]
fn non_hud_settings_are_saved_and_projected_from_rust_state() {
    let config_path = temporary_config_path("settings_sections");
    let state = AppState::new_with_config_path(
        UiConfig::default(),
        LiveCaptureService::new(LiveCaptureResources::default()),
        config_path.clone(),
    );

    assert!(
        state
            .update_interface_settings(
                Language::Japanese,
                true,
                ThemePreset::Tactical,
                AccentColor::Orange,
                UiDensity::Compact,
                true,
                false,
                48.0,
            )
            .expect("interface settings save")
    );
    assert!(
        state
            .update_capture_settings(
                "udp port 30196".to_owned(),
                Some("capture-device".to_owned()),
                true,
                true,
                true,
                true,
                45,
                HotkeyBinding::new(false, false, false, config::HotkeyKey::Insert),
            )
            .expect("capture settings save")
    );
    let mut hotkeys = state.global_hotkeys();
    hotkeys.set_binding(
        nte_dps_tool::storage::config::GlobalHotkeyAction::ToggleCapture,
        Some(nte_dps_tool::storage::config::HotkeyBinding::new(
            true,
            false,
            true,
            nte_dps_tool::storage::config::HotkeyKey::F8,
        )),
    );
    state
        .update_global_hotkeys(hotkeys)
        .expect("global hotkeys save");
    let settings_revision = state.settings_revision();
    let main_revision = state.0.presentation.main_revision.load(Ordering::Acquire);
    assert!(
        state
            .update_main_dps_display(MainDpsDisplayConfig {
                metrics: vec![
                    config::MainDpsMetric::TeamDps,
                    config::MainDpsMetric::Duration,
                ],
                attributions: vec![
                    config::MainDpsAttribution::Character,
                    config::MainDpsAttribution::MaxHpReduction,
                ],
            })
            .expect("main DPS display settings save")
    );
    assert_eq!(state.settings_revision(), settings_revision + 1);
    assert_eq!(
        state.0.presentation.main_revision.load(Ordering::Acquire),
        main_revision + 1
    );

    let snapshot = state.settings_snapshot();
    assert_eq!(snapshot.interface.language, Language::Japanese);
    assert!(snapshot.interface.dark_mode);
    assert_eq!(snapshot.interface.theme_preset, ThemePreset::Tactical);
    assert_eq!(snapshot.capture.bpf_filter, "udp port 30196");
    assert_eq!(
        snapshot.capture.manual_capture_device.as_deref(),
        Some("capture-device")
    );
    assert!(snapshot.capture.include_max_hp_reduction_in_total_damage);
    assert!(snapshot.capture.separate_reaction_damage);
    assert_eq!(snapshot.capture.auto_round_idle_seconds, 45);
    assert_eq!(
        state.0.live_capture.history_archive_policy(),
        HistoryArchivePolicy {
            requested_dps_time_mode: DpsTimeBasis::WallClock,
            separate_reaction_damage: true,
        },
        "successful settings persistence must update the capture-side round policy",
    );
    assert_eq!(
        snapshot.hotkeys.bindings[0]
            .binding
            .as_ref()
            .map(|binding| binding.key.as_str()),
        Some("F8")
    );
    assert_eq!(
        snapshot.main_dps.metrics,
        [MainDpsMetric::TeamDps, MainDpsMetric::Duration]
    );
    assert_eq!(
        snapshot.main_dps.attributions,
        [
            MainDpsAttribution::Character,
            MainDpsAttribution::MaxHpReduction
        ]
    );

    let saved: UiConfig =
        serde_json::from_str(&fs::read_to_string(&config_path).expect("saved UI config"))
            .expect("valid saved UI config");
    assert_eq!(saved.language, Language::Japanese);
    assert_eq!(saved.theme_preset, ThemePreset::Tactical);
    assert_eq!(saved.accent, AccentColor::Orange);
    assert_eq!(saved.capture_filter, "udp port 30196");
    assert!(saved.reduce_motion);
    assert!(saved.include_max_hp_reduction_in_total_damage);
    assert_eq!(
        saved.manual_capture_device.as_deref(),
        Some("capture-device")
    );
    assert_eq!(saved.dps_time_mode(), DpsTimeMode::RealTime);
    assert_eq!(
        saved.main_dps_display.metrics,
        [
            config::MainDpsMetric::TeamDps,
            config::MainDpsMetric::Duration,
        ]
    );
    assert_eq!(
        saved.main_dps_display.attributions,
        [
            config::MainDpsAttribution::Character,
            config::MainDpsAttribution::MaxHpReduction,
        ]
    );

    let restored = AppState::new_with_config_path(
        saved,
        LiveCaptureService::new(LiveCaptureResources::default()),
        config_path.clone(),
    );
    assert_eq!(
        restored.settings_snapshot().capture.bpf_filter,
        "udp port 30196"
    );
    assert_eq!(
        restored.settings_snapshot().main_dps.metrics,
        [MainDpsMetric::TeamDps, MainDpsMetric::Duration]
    );
    assert_eq!(
        restored.0.live_capture.history_archive_policy(),
        HistoryArchivePolicy {
            requested_dps_time_mode: DpsTimeBasis::WallClock,
            separate_reaction_damage: true,
        },
        "startup must seed the capture-side round policy from sanitized config",
    );

    fs::remove_dir_all(config_path.parent().expect("config parent"))
        .expect("remove temporary config");
}

#[test]
fn proxy_game_directory_survives_reload_and_explicit_clear() {
    let config_path = temporary_config_path("proxy_game_directory");
    let state = AppState::new_with_config_path(
        UiConfig::default(),
        LiveCaptureService::new(LiveCaptureResources::default()),
        config_path.clone(),
    );
    assert!(
        state
            .set_proxy_game_directory(Some("GAME/Win64".to_owned()))
            .unwrap()
    );
    assert!(
        !state
            .set_proxy_game_directory(Some("GAME/Win64".to_owned()))
            .unwrap()
    );
    let saved: UiConfig = serde_json::from_str(&fs::read_to_string(&config_path).unwrap()).unwrap();
    let restored = AppState::new_with_config_path(
        saved,
        LiveCaptureService::new(LiveCaptureResources::default()),
        config_path.clone(),
    );
    assert_eq!(
        restored.proxy_game_directory().as_deref(),
        Some("GAME/Win64")
    );
    assert!(restored.set_proxy_game_directory(None).unwrap());
    let cleared: UiConfig =
        serde_json::from_str(&fs::read_to_string(&config_path).unwrap()).unwrap();
    assert_eq!(cleared.mod_studio_proxy_game_directory, None);
    fs::remove_dir_all(config_path.parent().unwrap()).unwrap();
}
#[test]
fn mod_studio_loading_method_survives_state_reload() {
    let config_path = temporary_config_path("mod_studio_loading_method");
    let state = AppState::new_with_config_path(
        UiConfig::default(),
        LiveCaptureService::new(LiveCaptureResources::default()),
        config_path.clone(),
    );

    assert_eq!(
        state.mod_studio_loading_method(),
        ModStudioLoadingMethod::Proxy
    );
    assert!(
        state
            .set_mod_studio_loading_method(ModStudioLoadingMethod::Loader)
            .expect("save Mod Studio loading method")
    );

    let saved: UiConfig =
        serde_json::from_str(&fs::read_to_string(&config_path).expect("saved UI config"))
            .expect("valid saved UI config");
    let restored = AppState::new_with_config_path(
        saved,
        LiveCaptureService::new(LiveCaptureResources::default()),
        config_path.clone(),
    );
    assert_eq!(
        restored.mod_studio_loading_method(),
        ModStudioLoadingMethod::Loader
    );

    fs::remove_dir_all(config_path.parent().expect("config parent"))
        .expect("remove temporary config");
}

#[test]
fn module_order_is_saved_before_the_projection_changes() {
    let config_path = temporary_config_path("module_order");
    let state = AppState::new_with_config_path(
        UiConfig::default(),
        LiveCaptureService::new(LiveCaptureResources::default()),
        config_path.clone(),
    );
    let initial_revision = state.stream_revision();

    assert!(
        state
            .move_hud_module(HudModule::Title, HudModule::Characters, true)
            .expect("module order save")
    );
    assert_ne!(state.stream_revision(), initial_revision);
    assert_eq!(
        state
            .snapshot()
            .expect("healthy live-capture snapshot")
            .hud
            .config
            .module_order,
        [
            HudModule::Summary,
            HudModule::Status,
            HudModule::Characters,
            HudModule::Title,
            HudModule::Timeline,
        ]
    );

    let saved: UiConfig =
        serde_json::from_str(&fs::read_to_string(&config_path).expect("saved UI config"))
            .expect("valid saved UI config");
    assert_eq!(
        saved.hud.module_order,
        [
            HudModule::Summary,
            HudModule::Status,
            HudModule::Characters,
            HudModule::Title,
            HudModule::Timeline,
        ]
    );

    let revision = state.stream_revision();
    assert!(
        !state
            .move_hud_module(HudModule::Title, HudModule::Characters, true)
            .expect("same module order")
    );
    assert_eq!(state.stream_revision(), revision);

    fs::remove_dir_all(config_path.parent().expect("config parent"))
        .expect("remove temporary config");
}

#[test]
fn hud_width_is_sanitized_and_saved_before_projection_changes() {
    let config_path = temporary_config_path("hud_width");
    let state = AppState::new_with_config_path(
        UiConfig::default(),
        LiveCaptureService::new(LiveCaptureResources::default()),
        config_path.clone(),
    );
    let initial_revision = state.stream_revision();

    assert!(state.set_hud_width(u16::MAX).expect("HUD width save"));
    assert_eq!(
        state
            .snapshot()
            .expect("healthy live-capture snapshot")
            .hud
            .config
            .width,
        nte_dps_tool::storage::config::HUD_WIDTH_MAX
    );
    assert_ne!(state.stream_revision(), initial_revision);

    let saved: UiConfig =
        serde_json::from_str(&fs::read_to_string(&config_path).expect("saved UI config"))
            .expect("valid saved UI config");
    assert_eq!(
        saved.hud.width,
        nte_dps_tool::storage::config::HUD_WIDTH_MAX
    );

    fs::remove_dir_all(config_path.parent().expect("config parent"))
        .expect("remove temporary config");
}

#[test]
fn always_on_top_is_saved_before_projection_changes() {
    let config_path = temporary_config_path("always_on_top");
    let state = AppState::new_with_config_path(
        UiConfig::default(),
        LiveCaptureService::new(LiveCaptureResources::default()),
        config_path.clone(),
    );
    let initial_revision = state.stream_revision();

    assert!(state.set_always_on_top(false).expect("always-on-top save"));
    assert!(!state.always_on_top());
    assert_ne!(state.stream_revision(), initial_revision);

    let saved: UiConfig =
        serde_json::from_str(&fs::read_to_string(&config_path).expect("saved UI config"))
            .expect("valid saved UI config");
    assert_eq!(saved.hud_always_on_top, Some(false));
    assert!(!saved.always_on_top);
    assert_eq!(saved.main_dps_always_on_top, Some(true));

    let revision = state.stream_revision();
    assert!(
        !state
            .set_always_on_top(false)
            .expect("same-value always-on-top")
    );
    assert_eq!(state.stream_revision(), revision);

    fs::remove_dir_all(config_path.parent().expect("config parent"))
        .expect("remove temporary config");
}

#[test]
fn desktop_window_always_on_top_preferences_are_independent() {
    let config_path = temporary_config_path("independent_always_on_top");
    let state = AppState::new_with_config_path(
        UiConfig::default(),
        LiveCaptureService::new(LiveCaptureResources::default()),
        config_path.clone(),
    );

    assert!(
        state
            .set_window_always_on_top(DesktopWindowKind::MainDps, false)
            .expect("main DPS always-on-top save")
    );
    assert!(
        state
            .set_window_always_on_top(DesktopWindowKind::Console, true)
            .expect("Console always-on-top save")
    );

    assert!(!state.window_always_on_top(DesktopWindowKind::MainDps));
    assert!(state.window_always_on_top(DesktopWindowKind::Hud));
    assert!(state.window_always_on_top(DesktopWindowKind::Console));
    assert!(!state.window_always_on_top(DesktopWindowKind::AbyssValues));

    let saved: UiConfig =
        serde_json::from_str(&fs::read_to_string(&config_path).expect("saved UI config"))
            .expect("valid saved UI config");
    assert_eq!(saved.main_dps_always_on_top, Some(false));
    assert_eq!(saved.hud_always_on_top, Some(true));
    assert_eq!(saved.console_always_on_top, Some(true));
    assert_eq!(saved.abyss_values_always_on_top, Some(false));

    fs::remove_dir_all(config_path.parent().expect("config parent"))
        .expect("remove temporary config");
}

#[test]
fn hud_window_position_is_saved_without_changing_projection_revision() {
    let config_path = temporary_config_path("hud_window_position");
    let state = AppState::new_with_config_path(
        UiConfig::default(),
        LiveCaptureService::new(LiveCaptureResources::default()),
        config_path.clone(),
    );
    let initial_revision = state.stream_revision();

    assert!(
        state
            .set_hud_window_position([-1920, 84])
            .expect("HUD position save")
    );
    assert_eq!(state.hud_window_position(), Some([-1920, 84]));
    assert_eq!(state.stream_revision(), initial_revision);

    let saved: UiConfig =
        serde_json::from_str(&fs::read_to_string(&config_path).expect("saved UI config"))
            .expect("valid saved UI config");
    assert_eq!(saved.hud_window_position, Some([-1920, 84]));
    assert!(
        !state
            .set_hud_window_position([-1920, 84])
            .expect("same-value HUD position")
    );

    fs::remove_dir_all(config_path.parent().expect("config parent"))
        .expect("remove temporary config");
}

#[test]
fn console_geometry_is_saved_without_publishing_combat_or_settings_state() {
    let config_path = temporary_config_path("console_geometry");
    let state = AppState::new_with_config_path(
        UiConfig::default(),
        LiveCaptureService::new(LiveCaptureResources::default()),
        config_path.clone(),
    );
    let initial_stream_revision = state.stream_revision();
    let initial_settings_revision = state.settings_revision();

    assert!(
        state
            .set_console_window_geometry([1180.0, 760.0], [-1920.0, 84.0])
            .expect("Console geometry save")
    );
    assert_eq!(
        state.console_window_geometry(),
        (Some([1180.0, 760.0]), Some([-1920.0, 84.0]))
    );
    assert_eq!(state.stream_revision(), initial_stream_revision);
    assert_eq!(state.settings_revision(), initial_settings_revision);

    let saved: UiConfig =
        serde_json::from_str(&fs::read_to_string(&config_path).expect("saved UI config"))
            .expect("valid saved UI config");
    assert_eq!(saved.console_window_size, Some([1180.0, 760.0]));
    assert_eq!(saved.console_window_position, Some([-1920.0, 84.0]));
    assert!(
        !state
            .set_console_window_geometry([1180.0, 760.0], [-1920.0, 84.0])
            .expect("same-value Console geometry")
    );

    fs::remove_dir_all(config_path.parent().expect("config parent"))
        .expect("remove temporary config");
}

#[test]
fn combat_detail_columns_and_positions_persist_in_the_existing_ui_config() {
    let config_path = temporary_config_path("combat_detail_preferences");
    let state = AppState::new_with_config_path(
        UiConfig::default(),
        LiveCaptureService::new(LiveCaptureResources::default()),
        config_path.clone(),
    );
    let columns = nte_dps_tool::storage::config::HitDetailColumnsConfig {
        show_time: false,
        type_width: u16::MAX,
        ..Default::default()
    };
    assert!(
        state
            .set_hit_detail_columns(columns)
            .expect("detail columns save")
    );
    assert!(!state.ui_config_snapshot().hit_detail_columns.show_time);
    assert_eq!(
        state.ui_config_snapshot().hit_detail_columns.type_width,
        600
    );

    assert!(
        state
            .set_main_dps_detail_window_geometry(
                [920.0, 640.0],
                [120.0, 80.0],
                MainDpsDetailKind::Team,
            )
            .expect("team detail geometry save")
    );
    assert!(
        state
            .set_main_dps_detail_window_geometry(
                [840.0, 600.0],
                [240.0, 160.0],
                MainDpsDetailKind::Character,
            )
            .expect("character detail geometry save")
    );
    assert_eq!(
        state.main_dps_detail_window_geometry(MainDpsDetailKind::Character),
        (Some([840.0, 600.0]), Some([240.0, 160.0]))
    );

    let saved: UiConfig =
        serde_json::from_str(&fs::read_to_string(&config_path).expect("saved UI config"))
            .expect("valid saved UI config");
    assert_eq!(saved.team_hit_detail_window_position, Some([120.0, 80.0]));
    assert_eq!(saved.hit_detail_window_position, Some([240.0, 160.0]));
    assert_eq!(saved.team_hit_detail_window_size, Some([920.0, 640.0]));
    assert_eq!(saved.hit_detail_window_size, Some([840.0, 600.0]));

    fs::remove_dir_all(config_path.parent().expect("config parent"))
        .expect("remove temporary config");
}

#[test]
fn failed_module_visibility_save_keeps_the_previous_projection() {
    let config_path = temporary_config_path("module_visibility_failure");
    fs::create_dir(&config_path).expect("directory blocks config file replacement");
    let state = AppState::new_with_config_path(
        UiConfig::default(),
        LiveCaptureService::new(LiveCaptureResources::default()),
        config_path.clone(),
    );
    let initial_revision = state.stream_revision();

    assert!(
        state
            .set_hud_module_visibility(HudModule::Title, true)
            .is_err()
    );
    assert!(
        !state
            .snapshot()
            .expect("healthy live-capture snapshot")
            .hud
            .config
            .show_title
    );
    assert_eq!(state.stream_revision(), initial_revision);

    fs::remove_dir_all(config_path.parent().expect("config parent"))
        .expect("remove temporary config");
}

#[test]
fn failed_always_on_top_save_keeps_the_previous_projection() {
    let config_path = temporary_config_path("always_on_top_failure");
    fs::create_dir(&config_path).expect("directory blocks config file replacement");
    let state = AppState::new_with_config_path(
        UiConfig::default(),
        LiveCaptureService::new(LiveCaptureResources::default()),
        config_path.clone(),
    );
    let initial_revision = state.stream_revision();

    assert!(state.set_always_on_top(false).is_err());
    assert!(state.always_on_top());
    assert_eq!(state.stream_revision(), initial_revision);

    fs::remove_dir_all(config_path.parent().expect("config parent"))
        .expect("remove temporary config");
}

#[test]
fn failed_hud_window_position_save_keeps_the_previous_position() {
    let config_path = temporary_config_path("hud_window_position_failure");
    fs::create_dir(&config_path).expect("directory blocks config file replacement");
    let state = AppState::new_with_config_path(
        UiConfig::default(),
        LiveCaptureService::new(LiveCaptureResources::default()),
        config_path.clone(),
    );
    let initial_revision = state.stream_revision();

    assert!(state.set_hud_window_position([120, 80]).is_err());
    assert_eq!(state.hud_window_position(), None);
    assert_eq!(state.stream_revision(), initial_revision);

    fs::remove_dir_all(config_path.parent().expect("config parent"))
        .expect("remove temporary config");
}

#[test]
fn deleted_history_undo_token_is_opaque_and_single_use() {
    let state = AppState::default();
    let (directory, tombstone) = temporary_history_tombstone("opaque-single-use");
    let record_id = tombstone.record_id().to_owned();

    let token = state
        .remember_deleted_history_tombstone(tombstone)
        .expect("remember deleted History record");

    assert!(!token.contains(&record_id));
    assert_eq!(token.len(), 37);
    assert_eq!(
        state
            .peek_deleted_history_tombstone(&token)
            .expect("read deleted History record")
            .expect("active undo record")
            .record_id(),
        record_id
    );
    assert!(
        state
            .consume_deleted_history(&token)
            .expect("consume deleted History record")
    );
    assert!(
        state
            .peek_deleted_history_tombstone(&token)
            .expect("read consumed History undo")
            .is_none()
    );
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn replacing_and_expiring_history_undo_discards_old_tombstones_outside_the_slot() {
    let state = AppState::default();
    let (first_directory, first) = temporary_history_tombstone("replace-first");
    let (second_directory, second) = temporary_history_tombstone("replace-second");
    state
        .remember_deleted_history_tombstone(first)
        .expect("remember first History tombstone");
    let second_token = state
        .remember_deleted_history_tombstone(second)
        .expect("replace History tombstone");
    assert!(
        !first_directory.join(".nte-history-undo").exists(),
        "replacement must discard the previous on-disk tombstone"
    );
    {
        let mut undo = state.0.history.undo.lock().expect("History undo slot");
        undo.as_mut().expect("active second tombstone").expires_at =
            Instant::now() - Duration::from_millis(1);
    }
    assert!(
        state
            .peek_deleted_history_tombstone(&second_token)
            .expect("expire History tombstone")
            .is_none()
    );
    assert!(
        !second_directory.join(".nte-history-undo").exists(),
        "expiry must discard the on-disk tombstone"
    );
    let _ = fs::remove_dir_all(first_directory);
    let _ = fs::remove_dir_all(second_directory);
}

#[test]
fn stale_island_dismiss_keeps_the_newer_notice() {
    let state = AppState::default();
    let stale_id = state.publish_island_notice(IslandTone::Info, "First notice", Vec::new(), None);
    let current_id =
        state.publish_island_notice(IslandTone::Success, "Second notice", Vec::new(), None);
    let published_revision = state.0.island_notice.revision();

    assert!(!state.dismiss_island_notice(&stale_id));
    assert_eq!(state.0.island_notice.revision(), published_revision);
    assert_eq!(
        state.island_notice().map(|notice| notice.id),
        Some(current_id.clone())
    );
    assert_eq!(state.0.island_notice.revision(), published_revision);
    assert!(state.dismiss_island_notice(&current_id));
    assert_eq!(state.0.island_notice.revision(), published_revision + 1);
    assert!(state.island_notice().is_none());
    assert_eq!(state.0.island_notice.revision(), published_revision + 1);
}

#[test]
fn poisoned_island_notice_is_discarded_once_without_projecting_partial_data() {
    let state = AppState::default();
    state.publish_island_notice(IslandTone::Info, "Healthy notice", Vec::new(), None);
    let initial_revision = state.0.island_notice.revision();
    let poison_state = state.clone();
    let _ = std::thread::spawn(move || {
        let mut notice = poison_state
            .0
            .island_notice
            .notice
            .lock()
            .expect("island notice lock");
        notice.as_mut().expect("published notice").message_arguments =
            vec!["private-partial-notice".to_owned()];
        panic!("poison island notice");
    })
    .join();

    assert!(state.island_notice().is_none());
    assert!(!state.0.island_notice.notice.is_poisoned());
    assert_eq!(state.0.island_notice.revision(), initial_revision + 1);
    let serialized =
        serde_json::to_string(&crate::contract::island::IslandSnapshot::from_state(&state))
            .expect("serialize recovered island snapshot");
    assert!(!serialized.contains("private-partial-notice"));
    assert!(!serialized.contains("poison"));
    assert!(state.island_notice().is_none());
    assert_eq!(
        state.0.island_notice.revision(),
        initial_revision + 1,
        "repeated empty reads must not spam the revision"
    );
}

#[test]
fn poisoned_island_dismiss_reports_the_visible_clear_once() {
    let state = AppState::default();
    state.publish_island_notice(IslandTone::Info, "Healthy notice", Vec::new(), None);
    let initial_revision = state.0.island_notice.revision();
    let poison_state = state.clone();
    let _ = std::thread::spawn(move || {
        let _notice = poison_state
            .0
            .island_notice
            .notice
            .lock()
            .expect("island notice lock");
        panic!("poison island notice before dismiss");
    })
    .join();

    assert!(state.dismiss_island_notice("untrusted-notice-id"));
    assert_eq!(state.0.island_notice.revision(), initial_revision + 1);
    assert!(!state.dismiss_island_notice("untrusted-notice-id"));
    assert_eq!(state.0.island_notice.revision(), initial_revision + 1);
    assert!(state.island_notice().is_none());
}

#[test]
fn repeated_capture_notices_do_not_replay_stale_stops_or_suppress_a_new_run() {
    let state = AppState::default();
    let mut previous_stop: Option<String> = None;
    for _ in 0..3 {
        let start = state.publish_island_notice(
            IslandTone::Info,
            "Starting live capture...",
            Vec::new(),
            None,
        );
        if let Some(previous) = previous_stop {
            assert!(!state.dismiss_island_notice(&previous));
        }
        assert_eq!(state.island_notice().unwrap().id, start);
        state.publish_island_notice(
            IslandTone::Info,
            "Stopping live capture...",
            Vec::new(),
            None,
        );
        let stop =
            state.publish_island_notice(IslandTone::Success, "Capture stopped", Vec::new(), None);
        assert_eq!(
            state.publish_island_notice(IslandTone::Success, "Capture stopped", Vec::new(), None),
            stop
        );
        previous_stop = Some(stop);
    }
}

#[test]
fn duplicate_island_notice_does_not_restart_notice() {
    let runtime = IslandNoticeRuntime::default();
    let id = runtime.publish(
        IslandTone::Info,
        "Stopping live capture...",
        Vec::new(),
        None,
    );
    let first = runtime.current().unwrap();
    let revision = runtime.revision();
    assert_eq!(
        runtime.publish(
            IslandTone::Info,
            "Stopping live capture...",
            Vec::new(),
            None
        ),
        id
    );
    let second = runtime.current().unwrap();
    assert_eq!(second.expires_at, first.expires_at);
    assert_eq!(runtime.revision(), revision);
}

#[test]
fn island_notice_revision_tracks_only_publish_dismiss_and_expiry_changes() {
    let state = AppState::default();
    let initial_revision = state.0.island_notice.revision();
    let notice_id =
        state.publish_island_notice(IslandTone::Info, "Expiring notice", Vec::new(), None);
    assert_eq!(state.0.island_notice.revision(), initial_revision + 1);

    assert!(!state.dismiss_island_notice("stale-notice"));
    assert_eq!(state.0.island_notice.revision(), initial_revision + 1);
    {
        let mut notice = state
            .0
            .island_notice
            .notice
            .lock()
            .expect("island notice lock");
        notice.as_mut().expect("published notice").expires_at =
            Instant::now() - Duration::from_millis(1);
    }
    assert!(state.island_notice().is_none());
    assert_eq!(state.0.island_notice.revision(), initial_revision + 2);
    assert!(state.island_notice().is_none());
    assert!(!state.dismiss_island_notice(&notice_id));
    assert_eq!(state.0.island_notice.revision(), initial_revision + 2);
}

impl StreamRegistration {
    fn is_cancelled(&self) -> bool {
        self.stop.load(Ordering::Acquire)
    }
}

impl StreamRegistry {
    fn len(&self) -> Result<usize, StreamRegistryError> {
        Ok(self.lock_entries()?.len())
    }
}

impl IslandNoticeRuntime {
    fn revision(&self) -> u64 {
        self.revision.load(Ordering::Acquire)
    }
}

impl HistoryRoundIndex {
    pub(crate) fn for_test(id: &str, has_details: bool) -> Self {
        Self {
            id: id.to_owned(),
            display_time: id.to_owned(),
            abyss_floor: None,
            has_details,
        }
    }
}

impl AppState {
    pub(crate) fn poison_update_runtime_for_test(&self) {
        self.0.update_runtime.poison_for_test();
    }

    fn refresh_capture_devices_with(
        &self,
        enumerate: impl FnOnce() -> Result<Vec<CaptureDeviceSnapshot>, CoreError>,
    ) -> Result<(), CoreError> {
        self.apply_capture_device_refresh(self.0.settings.refresh_devices_with(enumerate, || {
            self.publish_settings_effects(SettingsMutationEffects::SETTINGS_AND_MAIN);
        }))
    }

    pub(crate) fn restore_live_state_for_test(
        &self,
        state: CombatState,
        source: CaptureQualitySource,
    ) {
        let _ = self.0.live_capture.restore_session(state, source);
    }

    pub(crate) fn stream_registry_len(&self) -> Result<usize, StreamRegistryError> {
        self.0.streams.len()
    }
}
