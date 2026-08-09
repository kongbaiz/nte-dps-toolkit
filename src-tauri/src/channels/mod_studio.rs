use std::{sync::atomic::Ordering, thread, time::Duration};

use nte_dps_tool::core::mod_studio::{
    ModStudioRuntimeEvent as CoreModStudioRuntimeEvent, ModStudioRuntimeLog,
    ModStudioRuntimeSnapshot,
};
use nte_dps_tool::platform::mods_plugin::probe_runtime_presence;
use tauri::{State, WebviewWindow, ipc::Channel};

use crate::{
    contract::{
        CommandError, SubscriptionReceipt,
        mod_studio::{
            MOD_STUDIO_CONTRACT_VERSION, ModStudioRuntimeBatchSnapshot,
            ModStudioRuntimeConnectionSnapshot, ModStudioRuntimeConnectionStatusSnapshot,
            ModStudioRuntimeEntrySnapshot, ModStudioRuntimeEvent,
        },
    },
    state::AppState,
    windows::console,
};

pub(crate) const MOD_STUDIO_RUNTIME_STREAM_INTERVAL_MS: u32 = 250;
const STREAM_KEY_PREFIX: &str = "mod-studio:";

#[tauri::command]
pub(crate) fn subscribe_mod_studio_runtime(
    subscription_id: String,
    on_event: Channel<ModStudioRuntimeEvent>,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<SubscriptionReceipt, CommandError> {
    validate_subscription_id(&subscription_id)?;
    console::validate_window(&window)?;

    let stream_key = stream_key(&subscription_id);
    let state = state.inner().clone();
    let stop = state.begin_stream(window.label().to_owned(), stream_key.clone());
    thread::spawn(move || {
        let mut cursor = RuntimeStreamCursor::default();
        while !stop.load(Ordering::Acquire) {
            let events = match state.poll_mod_studio_runtime() {
                Ok(snapshot) => cursor.ingest_connected(snapshot),
                Err(_) => match probe_runtime_presence() {
                    Ok(true) => cursor.ingest_unavailable(RuntimeConnectionStatus::LoaderPresent),
                    Ok(false) => cursor.ingest_unavailable(RuntimeConnectionStatus::Waiting),
                    Err(_) => cursor.ingest_unavailable(RuntimeConnectionStatus::ProbeFailed),
                },
            };
            for event in events {
                if on_event.send(event).is_err() {
                    state.finish_stream(&stream_key, &stop);
                    return;
                }
            }
            thread::sleep(Duration::from_millis(u64::from(
                MOD_STUDIO_RUNTIME_STREAM_INTERVAL_MS,
            )));
        }
        state.finish_stream(&stream_key, &stop);
    });

    Ok(SubscriptionReceipt {
        subscription_id,
        stream_interval_ms: MOD_STUDIO_RUNTIME_STREAM_INTERVAL_MS,
    })
}

#[tauri::command]
pub(crate) fn unsubscribe_mod_studio_runtime(
    subscription_id: String,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<(), CommandError> {
    validate_subscription_id(&subscription_id)?;
    console::validate_window(&window)?;
    state.stop_stream(&stream_key(&subscription_id));
    Ok(())
}

fn stream_key(subscription_id: &str) -> String {
    format!("{STREAM_KEY_PREFIX}{subscription_id}")
}

fn validate_subscription_id(subscription_id: &str) -> Result<(), CommandError> {
    let valid_length = (1..=64).contains(&subscription_id.len());
    let valid_characters = subscription_id
        .bytes()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, b'-' | b'_'));
    if valid_length && valid_characters {
        Ok(())
    } else {
        Err(CommandError::invalid_mod_runtime_subscription_id())
    }
}

#[derive(Default)]
struct RuntimeStreamCursor {
    generation: u64,
    status: Option<RuntimeConnectionStatus>,
    last_log_sequence: u64,
    last_event_sequence: u64,
    next_sequence: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RuntimeConnectionStatus {
    Connected,
    LoaderPresent,
    Waiting,
    ProbeFailed,
}

impl RuntimeStreamCursor {
    fn ingest_connected(
        &mut self,
        snapshot: ModStudioRuntimeSnapshot,
    ) -> Vec<ModStudioRuntimeEvent> {
        let mut events = Vec::new();
        if self.status != Some(RuntimeConnectionStatus::Connected) {
            self.start_generation();
            events.push(self.connection_event(RuntimeConnectionStatus::Connected));
        }

        let newest_log_sequence = snapshot
            .logs
            .iter()
            .map(|entry| entry.sequence)
            .max()
            .unwrap_or(0);
        let newest_event_sequence = snapshot
            .events
            .iter()
            .map(|entry| entry.sequence)
            .max()
            .unwrap_or(0);
        let log_sequence_reset =
            newest_log_sequence != 0 && newest_log_sequence < self.last_log_sequence;
        let event_sequence_reset =
            newest_event_sequence != 0 && newest_event_sequence < self.last_event_sequence;
        if log_sequence_reset || event_sequence_reset {
            self.start_generation();
            events.push(self.connection_event(RuntimeConnectionStatus::Connected));
        }

        let mut pending = snapshot
            .logs
            .into_iter()
            .filter(|entry| entry.sequence > self.last_log_sequence)
            .map(PendingRuntimeEntry::Log)
            .chain(
                snapshot
                    .events
                    .into_iter()
                    .filter(|entry| entry.sequence > self.last_event_sequence)
                    .map(PendingRuntimeEntry::Event),
            )
            .collect::<Vec<_>>();
        pending.sort_by_key(PendingRuntimeEntry::order_key);
        let entries = pending
            .into_iter()
            .map(|entry| {
                self.next_sequence = self.next_sequence.saturating_add(1).max(1);
                match entry {
                    PendingRuntimeEntry::Log(log) => {
                        ModStudioRuntimeEntrySnapshot::from_log(self.next_sequence, log)
                    }
                    PendingRuntimeEntry::Event(event) => {
                        ModStudioRuntimeEntrySnapshot::from_event(self.next_sequence, event)
                    }
                }
            })
            .collect::<Vec<_>>();
        self.last_log_sequence = self.last_log_sequence.max(newest_log_sequence);
        self.last_event_sequence = self.last_event_sequence.max(newest_event_sequence);
        if !entries.is_empty() {
            events.push(ModStudioRuntimeEvent::Batch(
                ModStudioRuntimeBatchSnapshot {
                    contract_version: MOD_STUDIO_CONTRACT_VERSION,
                    generation: self.generation.to_string(),
                    entries,
                },
            ));
        }
        events
    }

    fn ingest_unavailable(
        &mut self,
        status: RuntimeConnectionStatus,
    ) -> Vec<ModStudioRuntimeEvent> {
        if self.status == Some(status) {
            return Vec::new();
        }
        if self.generation == 0 {
            self.generation = 1;
        }
        self.status = Some(status);
        vec![self.connection_event(status)]
    }

    fn start_generation(&mut self) {
        self.generation = self.generation.saturating_add(1).max(1);
        self.status = Some(RuntimeConnectionStatus::Connected);
        self.last_log_sequence = 0;
        self.last_event_sequence = 0;
        self.next_sequence = 0;
    }

    fn connection_event(&self, status: RuntimeConnectionStatus) -> ModStudioRuntimeEvent {
        ModStudioRuntimeEvent::Connection(ModStudioRuntimeConnectionSnapshot {
            contract_version: MOD_STUDIO_CONTRACT_VERSION,
            generation: self.generation.to_string(),
            status: match status {
                RuntimeConnectionStatus::Connected => {
                    ModStudioRuntimeConnectionStatusSnapshot::Connected
                }
                RuntimeConnectionStatus::LoaderPresent => {
                    ModStudioRuntimeConnectionStatusSnapshot::LoaderPresent
                }
                RuntimeConnectionStatus::Waiting => {
                    ModStudioRuntimeConnectionStatusSnapshot::Waiting
                }
                RuntimeConnectionStatus::ProbeFailed => {
                    ModStudioRuntimeConnectionStatusSnapshot::ProbeFailed
                }
            },
        })
    }
}

enum PendingRuntimeEntry {
    Log(ModStudioRuntimeLog),
    Event(CoreModStudioRuntimeEvent),
}

impl PendingRuntimeEntry {
    fn order_key(&self) -> (u64, u8, u64) {
        match self {
            Self::Log(entry) => (entry.timestamp_100ns, 0, entry.sequence),
            Self::Event(entry) => (entry.timestamp_100ns, 1, entry.sequence),
        }
    }
}

#[cfg(test)]
mod tests {
    use nte_dps_tool::core::mod_studio::ModStudioRuntimeLevel;

    use super::*;

    fn log(sequence: u64, timestamp_100ns: u64) -> ModStudioRuntimeLog {
        ModStudioRuntimeLog {
            sequence,
            timestamp_100ns,
            mod_id: "runtime".to_owned(),
            level: ModStudioRuntimeLevel::Info,
            message: "Hot reload applied.".to_owned(),
            message_key: Some("Hot reload applied."),
            message_arguments: Vec::new(),
        }
    }

    fn event(sequence: u64, timestamp_100ns: u64) -> CoreModStudioRuntimeEvent {
        CoreModStudioRuntimeEvent {
            sequence,
            timestamp_100ns,
            mod_id: "telemetry".to_owned(),
            name: "post.sample".to_owned(),
            values: vec![sequence],
        }
    }

    fn snapshot(logs: &[(u64, u64)], events: &[(u64, u64)]) -> ModStudioRuntimeSnapshot {
        ModStudioRuntimeSnapshot {
            logs: logs
                .iter()
                .map(|(sequence, timestamp)| log(*sequence, *timestamp))
                .collect(),
            events: events
                .iter()
                .map(|(sequence, timestamp)| event(*sequence, *timestamp))
                .collect(),
        }
    }

    #[test]
    fn subscription_id_is_bounded_and_uses_only_stable_ascii() {
        assert!(validate_subscription_id("mod-runtime_01").is_ok());
        assert!(validate_subscription_id("").is_err());
        assert!(validate_subscription_id("mod/runtime").is_err());
        assert!(validate_subscription_id(&"a".repeat(65)).is_err());
    }

    #[test]
    fn stream_announces_connection_and_deduplicates_native_history() {
        let mut cursor = RuntimeStreamCursor::default();

        let first = cursor.ingest_connected(snapshot(&[(4, 40), (5, 50)], &[(2, 45)]));
        let repeated = cursor.ingest_connected(snapshot(&[(4, 40), (5, 50)], &[(2, 45)]));

        assert_eq!(first.len(), 2);
        assert!(matches!(first[0], ModStudioRuntimeEvent::Connection(_)));
        let ModStudioRuntimeEvent::Batch(batch) = &first[1] else {
            panic!("initial history is emitted as a batch");
        };
        assert_eq!(batch.entries.len(), 3);
        assert!(matches!(
            batch.entries[0],
            ModStudioRuntimeEntrySnapshot::Log { .. }
        ));
        assert!(matches!(
            batch.entries[1],
            ModStudioRuntimeEntrySnapshot::Event { .. }
        ));
        assert!(repeated.is_empty());
    }

    #[test]
    fn reconnect_and_sequence_reset_start_new_generations() {
        let mut cursor = RuntimeStreamCursor::default();
        cursor.ingest_connected(snapshot(&[(8, 80)], &[(6, 60)]));
        let disconnected = cursor.ingest_unavailable(RuntimeConnectionStatus::Waiting);
        let reconnected = cursor.ingest_connected(snapshot(&[(8, 80)], &[(6, 60)]));
        let reset = cursor.ingest_connected(snapshot(&[(1, 10)], &[(1, 11)]));

        assert_eq!(disconnected.len(), 1);
        assert_eq!(reconnected.len(), 2);
        assert_eq!(reset.len(), 2);
        let ModStudioRuntimeEvent::Connection(connection) = &reset[0] else {
            panic!("sequence reset starts with a connection snapshot");
        };
        assert_eq!(connection.generation, "3");
    }

    #[test]
    fn loaded_runtime_without_a_game_hook_is_distinct_from_waiting() {
        let mut cursor = RuntimeStreamCursor::default();

        let loaded = cursor.ingest_unavailable(RuntimeConnectionStatus::LoaderPresent);
        let repeated = cursor.ingest_unavailable(RuntimeConnectionStatus::LoaderPresent);
        let waiting = cursor.ingest_unavailable(RuntimeConnectionStatus::Waiting);

        let ModStudioRuntimeEvent::Connection(loaded) = &loaded[0] else {
            panic!("loader presence is emitted as a connection snapshot");
        };
        assert_eq!(
            loaded.status,
            ModStudioRuntimeConnectionStatusSnapshot::LoaderPresent
        );
        assert!(repeated.is_empty());
        let ModStudioRuntimeEvent::Connection(waiting) = &waiting[0] else {
            panic!("waiting is emitted as a connection snapshot");
        };
        assert_eq!(
            waiting.status,
            ModStudioRuntimeConnectionStatusSnapshot::Waiting
        );
    }
}
