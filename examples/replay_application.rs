//! Exercises the real PCAP import -> EngineEvent -> shared reducer path.
use anyhow::{Context, Result};
use nte_dps_tool::{
    core::reducer::{CoreSignal, apply_engine_event},
    engine::{
        capture::{CaptureResources, import_pcapng},
        model::{CombatState, EngineEvent},
        parser::{AbilityCatalog, CHARACTER_DATA_PATH, load_characters},
    },
};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
};

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    anyhow::ensure!(
        args.len() == 2,
        "usage: replay_application PCAPNG OUTPUT_JSON; requires NTE_EXACT_PACKET_CONFIG"
    );
    let characters = load_characters(Path::new(CHARACTER_DATA_PATH))?;
    let resources = CaptureResources {
        characters: Arc::new(characters),
        ability_catalog: Arc::new(AbilityCatalog::default()),
    };
    let (sender, receiver) = crossbeam_channel::unbounded();
    let handle = import_pcapng(
        PathBuf::from(&args[0]),
        resources,
        None,
        true,
        false,
        sender,
        Arc::new(AtomicBool::new(false)),
    )?;
    let mut state = CombatState::default();
    let mut errors = Vec::new();
    let mut changes = 0;
    let mut noops = 0;
    let mut kinds = HashMap::<String, usize>::new();
    for event in receiver {
        let stop = matches!(event, EngineEvent::CaptureStopped);
        let kind = match &event {
            EngineEvent::ExactSettlement(_) => "exact",
            EngineEvent::Hit(_) => "legacy_hit",
            EngineEvent::HitFollowUp(_) => "legacy_followup",
            EngineEvent::HitDamageCorrection(_) => "legacy_correction",
            EngineEvent::UnattributedServerDamage(_) => "legacy_unattributed",
            _ => "other",
        };
        *kinds.entry(kind.into()).or_default() += 1;
        match apply_engine_event(&mut state, event) {
            CoreSignal::Error(e) => errors.push(e),
            CoreSignal::StateChanged => changes += 1,
            CoreSignal::Unchanged => noops += 1,
            _ => {}
        }
        if stop {
            break;
        }
    }
    handle
        .join()
        .map_err(|_| anyhow::anyhow!("import_worker_panicked"))?;
    anyhow::ensure!(errors.is_empty(), "import_errors: {errors:?}");
    anyhow::ensure!(
        state.hits.iter().all(|h| h.exact.is_some()),
        "legacy_hit_leaked"
    );
    let output = serde_json::json!({"hits":state.hits,"totalDamage":state.total_damage,"totalDamageTaken":state.total_damage_taken,"stateChanges":changes,"noops":noops,"eventKinds":kinds});
    let bytes = serde_json::to_vec_pretty(&output)?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&args[1])?;
    std::io::Write::write_all(&mut file, &bytes)?;
    let back: serde_json::Value = serde_json::from_slice(&std::fs::read(&args[1])?)?;
    anyhow::ensure!(back == output, "readback_mismatch");
    println!(
        "hits={} outgoing={} incoming={}",
        output["hits"].as_array().context("hits")?.len(),
        state.total_damage,
        state.total_damage_taken
    );
    Ok(())
}
