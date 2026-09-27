//! Offline PCAP -> the real capture decoder/reducer. No plugin snapshot input.
use anyhow::{Result, ensure};
use nte_dps_tool::{
    core::reducer::{CoreSignal, apply_engine_event},
    engine::{
        capture::{CaptureResources, import_pcapng},
        model::{CombatState, EngineEvent},
        parser::{AbilityCatalog, CHARACTER_DATA_PATH, load_characters},
    },
};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
};
fn main() -> Result<()> {
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    ensure!(
        args.len() == 2,
        "usage: replay_equipment PCAPNG OUTPUT_JSON"
    );
    let resources = CaptureResources {
        characters: Arc::new(load_characters(Path::new(CHARACTER_DATA_PATH))?),
        ability_catalog: Arc::new(AbilityCatalog::default()),
    };
    let (tx, rx) = crossbeam_channel::bounded(256);
    let worker = import_pcapng(
        PathBuf::from(&args[0]),
        resources,
        None,
        true,
        false,
        tx,
        Arc::new(AtomicBool::new(false)),
    )?;
    let mut state = CombatState::default();
    let mut errors = Vec::new();
    let mut inventory_warnings = Vec::new();
    let mut inventory_events = 0;
    let mut character_events = 0;
    for event in rx {
        if let EngineEvent::Warning(code) = &event
            && code.starts_with("exact_inventory_")
        {
            inventory_warnings.push(code.clone());
        }
        let stopped = matches!(event, EngineEvent::CaptureStopped);
        inventory_events += usize::from(matches!(
            event,
            EngineEvent::EmptyCurtain(_) | EngineEvent::PacketInventory { .. }
        ));
        character_events += usize::from(matches!(
            event,
            EngineEvent::EmptyCurtainCharacters(_) | EngineEvent::PacketInventory { .. }
        ));
        if let CoreSignal::Error(error) = apply_engine_event(&mut state, event) {
            errors.push(error);
        }
        if stopped {
            break;
        }
    }
    worker
        .join()
        .map_err(|_| anyhow::anyhow!("inventory replay worker panicked"))?;
    let result = serde_json::json!({"items":state.empty_curtain,"characters":state.empty_curtain_characters,"inventoryEvents":inventory_events,"characterEvents":character_events,"errors":errors,"inventoryWarnings":inventory_warnings});
    let bytes = serde_json::to_vec_pretty(&result)?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&args[1])?;
    std::io::Write::write_all(&mut file, &bytes)?;
    let check: serde_json::Value = serde_json::from_slice(&std::fs::read(&args[1])?)?;
    ensure!(check == result, "readback mismatch");
    println!(
        "items={} characters={} inventory_events={} character_events={} errors={} inventory_warnings={}",
        state.empty_curtain.len(),
        state.empty_curtain_characters.len(),
        inventory_events,
        character_events,
        errors.len(),
        inventory_warnings.len()
    );
    Ok(())
}
