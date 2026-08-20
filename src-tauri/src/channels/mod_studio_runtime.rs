use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use nte_dps_tool::{
    core::mod_studio::{ModStudioRuntimeEvent, ModStudioRuntimeLog, ModStudioRuntimeSnapshot},
    platform::mods_plugin::{
        ModsPluginRuntimePresence, ModsPluginRuntimeProbeError, ModsPluginRuntimeProbeErrorCode,
        probe_runtime_presence,
    },
};
use tauri::async_runtime::JoinHandle;
use tokio::time::{MissedTickBehavior, interval};

use crate::state::AppState;

const MOD_STUDIO_POLL_INTERVAL: Duration = Duration::from_secs(1);
const MOD_STUDIO_TRANSIENT_MISS_GRACE: u8 = 3;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ModStudioPollState {
    Connected(Arc<ModStudioRuntimeSnapshot>),
    LoaderPresent,
    Waiting,
    AcknowledgementRequired,
    ProbeFailed(ModsPluginRuntimeProbeError),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ModStudioPollObservation {
    pub(crate) generation: u64,
    pub(crate) state: ModStudioPollState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ModStudioPollRuntimeError {
    Unavailable,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum ModStudioPollTarget {
    #[default]
    Logs,
    Events,
}

enum ModStudioPollResult {
    Logs(Vec<ModStudioRuntimeLog>),
    Events(Vec<ModStudioRuntimeEvent>),
}

struct ModStudioPollSession {
    next_target: ModStudioPollTarget,
    snapshot: ModStudioRuntimeSnapshot,
    last_connected: Option<Arc<ModStudioRuntimeSnapshot>>,
    transient_misses: u8,
}

impl Default for ModStudioPollSession {
    fn default() -> Self {
        Self {
            next_target: ModStudioPollTarget::Logs,
            snapshot: ModStudioRuntimeSnapshot {
                logs: Vec::new(),
                events: Vec::new(),
            },
            last_connected: None,
            transient_misses: 0,
        }
    }
}

impl ModStudioPollSession {
    fn poll(&mut self, state: &AppState) -> ModStudioPollState {
        let result = match self.next_target {
            ModStudioPollTarget::Logs => state
                .poll_mod_studio_runtime_logs()
                .map(ModStudioPollResult::Logs),
            ModStudioPollTarget::Events => state
                .poll_mod_studio_runtime_events()
                .map(ModStudioPollResult::Events),
        };
        match result {
            Ok(result) => self.record_success(result),
            Err(_) => self.record_failure(
                probe_runtime_presence(),
                state.mod_studio_risk_acknowledged(),
            ),
        }
    }

    fn record_success(&mut self, result: ModStudioPollResult) -> ModStudioPollState {
        match result {
            ModStudioPollResult::Logs(logs) => {
                self.snapshot.logs = logs;
                self.next_target = ModStudioPollTarget::Events;
            }
            ModStudioPollResult::Events(events) => {
                self.snapshot.events = events;
                self.next_target = ModStudioPollTarget::Logs;
            }
        }
        self.transient_misses = 0;
        let snapshot = Arc::new(self.snapshot.clone());
        self.last_connected = Some(Arc::clone(&snapshot));
        ModStudioPollState::Connected(snapshot)
    }

    fn record_failure(
        &mut self,
        presence: Result<ModsPluginRuntimePresence, ModsPluginRuntimeProbeError>,
        risk_acknowledged: bool,
    ) -> ModStudioPollState {
        if matches!(
            presence,
            Ok(ModsPluginRuntimePresence::Ready | ModsPluginRuntimePresence::Initializing)
        ) && self.transient_misses < MOD_STUDIO_TRANSIENT_MISS_GRACE
            && let Some(snapshot) = self.last_connected.as_ref()
        {
            self.transient_misses += 1;
            return ModStudioPollState::Connected(Arc::clone(snapshot));
        }

        self.snapshot.logs.clear();
        self.snapshot.events.clear();
        self.last_connected = None;
        self.transient_misses = 0;
        classify_runtime_presence(presence, risk_acknowledged)
    }
}

#[derive(Default)]
struct ModStudioPollCache {
    generation: u64,
    latest: Option<ModStudioPollState>,
}

struct ModStudioPollShared {
    healthy: AtomicBool,
    cache: Mutex<ModStudioPollCache>,
}

impl Default for ModStudioPollShared {
    fn default() -> Self {
        Self {
            healthy: AtomicBool::new(true),
            cache: Mutex::new(ModStudioPollCache::default()),
        }
    }
}

#[derive(Clone)]
pub(crate) struct ModStudioMonitorHandle {
    shared: Arc<ModStudioPollShared>,
}

impl ModStudioMonitorHandle {
    pub(crate) fn observe(
        &self,
    ) -> Result<Option<ModStudioPollObservation>, ModStudioPollRuntimeError> {
        if !self.shared.healthy.load(Ordering::Acquire) {
            return Err(ModStudioPollRuntimeError::Unavailable);
        }
        let cache = self.shared.cache.lock().map_err(|mut poison| {
            **poison.get_mut() = ModStudioPollCache::default();
            self.shared.cache.clear_poison();
            self.shared.healthy.store(false, Ordering::Release);
            ModStudioPollRuntimeError::Unavailable
        })?;
        Ok(cache.latest.clone().map(|state| ModStudioPollObservation {
            generation: cache.generation,
            state,
        }))
    }
}

/// Application-owned async poller. Native IPC runs on Tokio's blocking lane;
/// subscriptions only read the latest immutable observation.
pub(crate) struct ModStudioMonitorRuntime {
    stop: Arc<AtomicBool>,
    shared: Arc<ModStudioPollShared>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl ModStudioMonitorRuntime {
    pub(crate) fn start(state: AppState) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let shared = Arc::new(ModStudioPollShared::default());
        let task_stop = Arc::clone(&stop);
        let task_shared = Arc::clone(&shared);
        let worker = tauri::async_runtime::spawn(async move {
            let mut ticker = interval(MOD_STUDIO_POLL_INTERVAL);
            let mut poll_session = ModStudioPollSession::default();
            ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
            while !task_stop.load(Ordering::Acquire) {
                ticker.tick().await;
                if task_stop.load(Ordering::Acquire) {
                    break;
                }
                let poll_state = state.clone();
                let session = poll_session;
                let next = tauri::async_runtime::spawn_blocking(move || {
                    let mut session = session;
                    let next = catch_unwind(AssertUnwindSafe(|| session.poll(&poll_state)))
                        .unwrap_or(ModStudioPollState::ProbeFailed(
                            ModsPluginRuntimeProbeError {
                                code: ModsPluginRuntimeProbeErrorCode::PollWorkerFailed,
                                os_error_code: None,
                            },
                        ));
                    (session, next)
                })
                .await;
                let Ok((next_session, next)) = next else {
                    task_shared.healthy.store(false, Ordering::Release);
                    break;
                };
                poll_session = next_session;
                match task_shared.cache.lock() {
                    Ok(mut cache) => {
                        cache.generation = cache.generation.saturating_add(1).max(1);
                        cache.latest = Some(next);
                    }
                    Err(_) => {
                        task_shared.healthy.store(false, Ordering::Release);
                        break;
                    }
                }
            }
        });
        Self {
            stop,
            shared,
            worker: Mutex::new(Some(worker)),
        }
    }

    pub(crate) fn handle(&self) -> ModStudioMonitorHandle {
        ModStudioMonitorHandle {
            shared: Arc::clone(&self.shared),
        }
    }

    pub(crate) fn shutdown(&self) -> bool {
        self.stop.store(true, Ordering::Release);
        let worker = match self.worker.lock() {
            Ok(mut worker) => worker.take(),
            Err(mut poison) => {
                let worker = poison.get_mut().take();
                self.worker.clear_poison();
                worker
            }
        };
        if let Some(worker) = &worker {
            worker.abort();
        }
        worker.is_some()
    }
}

impl Drop for ModStudioMonitorRuntime {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn classify_runtime_presence(
    presence: Result<ModsPluginRuntimePresence, ModsPluginRuntimeProbeError>,
    risk_acknowledged: bool,
) -> ModStudioPollState {
    match presence {
        Ok(ModsPluginRuntimePresence::Ready) => ModStudioPollState::LoaderPresent,
        Ok(ModsPluginRuntimePresence::Initializing) => ModStudioPollState::Waiting,
        Ok(ModsPluginRuntimePresence::Absent) if !risk_acknowledged => {
            ModStudioPollState::AcknowledgementRequired
        }
        Ok(ModsPluginRuntimePresence::Absent) => ModStudioPollState::Waiting,
        Err(error) => ModStudioPollState::ProbeFailed(error),
    }
}

#[cfg(test)]
mod tests {
    use nte_dps_tool::core::mod_studio::ModStudioRuntimeLevel;

    use super::*;

    #[test]
    fn runtime_poll_alternates_single_ipc_queries_without_dropping_cached_data() {
        let mut session = ModStudioPollSession::default();
        let log = ModStudioRuntimeLog {
            sequence: 1,
            timestamp_100ns: 2,
            mod_id: "fixture".to_owned(),
            level: ModStudioRuntimeLevel::Info,
            message: "loaded".to_owned(),
            message_key: None,
            message_arguments: Vec::new(),
        };
        let event = ModStudioRuntimeEvent {
            sequence: 3,
            timestamp_100ns: 4,
            mod_id: "fixture".to_owned(),
            name: "tick".to_owned(),
            values: vec![5],
        };
        assert_eq!(session.next_target, ModStudioPollTarget::Logs);

        let ModStudioPollState::Connected(snapshot) =
            session.record_success(ModStudioPollResult::Logs(vec![log.clone()]))
        else {
            panic!("log query must establish a connected snapshot");
        };
        assert_eq!(snapshot.logs, vec![log.clone()]);
        assert!(snapshot.events.is_empty());
        assert_eq!(session.next_target, ModStudioPollTarget::Events);
        let ModStudioPollState::Connected(snapshot) =
            session.record_success(ModStudioPollResult::Events(vec![event.clone()]))
        else {
            panic!("event query must keep the runtime connected");
        };
        assert_eq!(snapshot.logs, vec![log]);
        assert_eq!(snapshot.events, vec![event]);
        assert_eq!(session.next_target, ModStudioPollTarget::Logs);
    }

    #[test]
    fn connected_runtime_ignores_short_pipe_rearm_gaps() {
        let mut session = ModStudioPollSession::default();
        let _ = session.record_success(ModStudioPollResult::Logs(Vec::new()));

        for _ in 0..MOD_STUDIO_TRANSIENT_MISS_GRACE {
            assert!(matches!(
                session.record_failure(Ok(ModsPluginRuntimePresence::Initializing), true),
                ModStudioPollState::Connected(_)
            ));
        }
        assert_eq!(
            session.record_failure(Ok(ModsPluginRuntimePresence::Initializing), true),
            ModStudioPollState::Waiting
        );
    }

    #[test]
    fn transient_runtime_presence_states_remain_waiting() {
        assert_eq!(
            classify_runtime_presence(Ok(ModsPluginRuntimePresence::Initializing), true),
            ModStudioPollState::Waiting
        );
        assert_eq!(
            classify_runtime_presence(Ok(ModsPluginRuntimePresence::Absent), true),
            ModStudioPollState::Waiting
        );
    }

    #[test]
    fn absent_runtime_requires_acknowledgement_before_waiting() {
        assert_eq!(
            classify_runtime_presence(Ok(ModsPluginRuntimePresence::Absent), false),
            ModStudioPollState::AcknowledgementRequired
        );
    }

    #[test]
    fn runtime_presence_access_denied_remains_probe_failed() {
        let error = ModsPluginRuntimeProbeError {
            code: ModsPluginRuntimeProbeErrorCode::RuntimeEventAccessDenied,
            os_error_code: Some(5),
        };
        assert_eq!(
            classify_runtime_presence(Err(error), true),
            ModStudioPollState::ProbeFailed(error)
        );
    }
}
