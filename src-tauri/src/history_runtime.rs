use std::{
    io,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, RecvTimeoutError, SyncSender, TryRecvError, TrySendError, sync_channel},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use nte_dps_tool::storage::history::load_history_summaries;
use nte_dps_tool::storage::i18n::{self, Language};
use tauri::ipc::Channel;

use crate::{
    channels::stream_runtime::serialize_stream_events,
    contract::{
        CommandError,
        history::{HistoryEvent, HistorySnapshot},
        stream::{StreamKind, StreamReadySignal},
    },
    state::{AppState, HistoryRuntimeError, StreamRegistration},
};

const HISTORY_RUNTIME_CONTROL_CAPACITY: usize = 128;
const MAX_HISTORY_STREAM_TASKS: usize = 256;
const HISTORY_RUNTIME_CANCELLATION_SCAN: Duration = Duration::from_millis(25);
const HISTORY_MAINTENANCE_INTERVAL: Duration = Duration::from_millis(250);
const HISTORY_REGISTRATION_ACK_TIMEOUT: Duration = Duration::from_secs(1);
pub(crate) const HISTORY_STREAM_INTERVAL: Duration = Duration::from_millis(250);

pub(crate) fn history_runtime_unavailable(_error: HistoryRuntimeError) -> CommandError {
    log::warn!("History operation did not finish after a runtime interruption");
    CommandError::history(
        "history_runtime_unavailable",
        "History operation did not finish.",
    )
}

#[cfg(test)]
pub(crate) fn run_history_maintenance_worker(
    state: &AppState,
    stop: &AtomicBool,
    mut wait: impl FnMut(),
) -> Result<(), HistoryRuntimeError> {
    while !stop.load(Ordering::Acquire) {
        state.maintain_history_rounds()?;
        wait();
    }
    Ok(())
}

pub(crate) struct HistoryRuntime {
    stop: Arc<AtomicBool>,
    control: SyncSender<HistoryRuntimeCommand>,
    worker: Mutex<Option<JoinHandle<()>>>,
    projection_cache: Arc<HistoryProjectionCache>,
}

pub(crate) struct HistoryStreamRegistration {
    pub(crate) registration: StreamRegistration,
    pub(crate) subscription_id: String,
    pub(crate) on_ready: Channel<StreamReadySignal>,
}

enum HistoryRuntimeCommand {
    Register {
        stream: HistoryStreamRegistration,
        acknowledged: SyncSender<bool>,
    },
    Wake,
    Shutdown,
}

struct HistoryStreamTask {
    registration: StreamRegistration,
    subscription_id: String,
    on_ready: Channel<StreamReadySignal>,
    last_projection_key: Option<HistoryProjectionKey>,
    next_tick: Instant,
}

/// Capacity is exactly one immutable, serialized History delivery. Serialization
/// enforces `MAX_STREAM_DELIVERY_BYTES`, so the cache cannot retain an unbounded
/// collection or payload. Lookup/publish hold this lock only long enough to
/// clone/swap an `Arc`; disk I/O, projection, serialization, channel send, and
/// worker join always happen outside it.
#[derive(Default)]
struct HistoryProjectionCache {
    entry: Mutex<Option<Arc<CachedHistoryProjection>>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct HistoryProjectionKey {
    revision: u64,
    language: Language,
    character_catalog_identity: usize,
}

struct CachedHistoryProjection {
    key: HistoryProjectionKey,
    delivery: Arc<Vec<u8>>,
}

impl HistoryProjectionCache {
    fn lookup(
        &self,
        key: HistoryProjectionKey,
    ) -> Result<Option<Arc<CachedHistoryProjection>>, HistoryRuntimeError> {
        let entry = match self.entry.lock() {
            Ok(entry) => entry,
            Err(mut poison) => {
                **poison.get_mut() = None;
                self.entry.clear_poison();
                drop(poison);
                return Err(HistoryRuntimeError::Unavailable);
            }
        };
        Ok(entry.as_ref().filter(|entry| entry.key == key).cloned())
    }

    fn publish(
        &self,
        projection: CachedHistoryProjection,
    ) -> Result<Arc<CachedHistoryProjection>, HistoryRuntimeError> {
        let mut entry = match self.entry.lock() {
            Ok(entry) => entry,
            Err(mut poison) => {
                **poison.get_mut() = None;
                self.entry.clear_poison();
                drop(poison);
                return Err(HistoryRuntimeError::Unavailable);
            }
        };
        if let Some(cached) = entry.as_ref().filter(|cached| cached.key == projection.key) {
            return Ok(Arc::clone(cached));
        }
        let projection = Arc::new(projection);
        *entry = Some(Arc::clone(&projection));
        Ok(projection)
    }

    #[cfg(test)]
    fn get_or_build(
        &self,
        key: HistoryProjectionKey,
        build: impl FnOnce() -> Result<CachedHistoryProjection, HistoryRuntimeError>,
    ) -> Result<Arc<CachedHistoryProjection>, HistoryRuntimeError> {
        if let Some(projection) = self.lookup(key)? {
            return Ok(projection);
        }
        self.publish(build()?)
    }

    fn clear(&self) {
        match self.entry.lock() {
            Ok(mut entry) => *entry = None,
            Err(mut poison) => {
                **poison.get_mut() = None;
                self.entry.clear_poison();
            }
        }
    }

    #[cfg(test)]
    fn poison_for_test(&self) {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _entry = self.entry.lock().expect("lock History projection cache");
            panic!("poison History projection cache fixture");
        }));
    }

    #[cfg(test)]
    fn entry_count_for_test(&self) -> usize {
        self.entry
            .lock()
            .expect("lock healthy History projection cache")
            .iter()
            .count()
    }
}

#[cfg(test)]
impl HistoryProjectionKey {
    fn fixture(revision: u64, language: Language, character_catalog_identity: usize) -> Self {
        Self {
            revision,
            language,
            character_catalog_identity,
        }
    }
}

impl HistoryRuntime {
    pub(crate) fn start(state: AppState) -> io::Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let (control, receiver) = sync_channel(HISTORY_RUNTIME_CONTROL_CAPACITY);
        let worker_stop = Arc::clone(&stop);
        let projection_cache = Arc::new(HistoryProjectionCache::default());
        let worker_projection_cache = Arc::clone(&projection_cache);
        let worker = thread::Builder::new()
            .name("nte-history-runtime".to_owned())
            .spawn(move || {
                run_history_runtime(state, worker_stop, receiver, worker_projection_cache)
            })?;
        Ok(Self {
            stop,
            control,
            worker: Mutex::new(Some(worker)),
            projection_cache,
        })
    }

    pub(crate) fn unavailable() -> Self {
        let (control, receiver) = sync_channel(1);
        drop(receiver);
        Self {
            stop: Arc::new(AtomicBool::new(true)),
            control,
            worker: Mutex::new(None),
            projection_cache: Arc::new(HistoryProjectionCache::default()),
        }
    }

    pub(crate) fn register_stream(&self, stream: HistoryStreamRegistration) -> Result<(), ()> {
        if self.stop.load(Ordering::Acquire) {
            return Err(());
        }
        let (acknowledged, acknowledgement) = sync_channel(1);
        match self.control.try_send(HistoryRuntimeCommand::Register {
            stream,
            acknowledged,
        }) {
            Ok(()) => match acknowledgement.recv_timeout(HISTORY_REGISTRATION_ACK_TIMEOUT) {
                Ok(true) if !self.stop.load(Ordering::Acquire) => Ok(()),
                Ok(true) => Err(()),
                Ok(false) | Err(_) => Err(()),
            },
            Err(TrySendError::Full(command) | TrySendError::Disconnected(command)) => {
                drop(command);
                Err(())
            }
        }
    }

    pub(crate) fn wake(&self) {
        match self.control.try_send(HistoryRuntimeCommand::Wake) {
            Ok(()) | Err(TrySendError::Full(_)) => {}
            Err(TrySendError::Disconnected(_)) => self.stop.store(true, Ordering::Release),
        }
    }

    /// Idempotently cancels and joins the single owned history worker. The
    /// handle is detached from the mutex before joining.
    pub(crate) fn shutdown(&self) -> bool {
        self.stop.store(true, Ordering::Release);
        let _ = self.control.try_send(HistoryRuntimeCommand::Shutdown);
        let worker = match self.worker.lock() {
            Ok(mut worker) => worker.take(),
            Err(mut poison) => {
                let worker = poison.get_mut().take();
                self.worker.clear_poison();
                drop(poison);
                worker
            }
        };
        let stopped = worker.is_some();
        if let Some(worker) = worker {
            let _ = worker.join();
        }
        self.projection_cache.clear();
        stopped
    }
}

impl Drop for HistoryRuntime {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn run_history_runtime(
    state: AppState,
    stop: Arc<AtomicBool>,
    receiver: Receiver<HistoryRuntimeCommand>,
    projection_cache: Arc<HistoryProjectionCache>,
) {
    let mut streams = Vec::<HistoryStreamTask>::new();
    let mut next_maintenance = Instant::now();

    while !stop.load(Ordering::Acquire) {
        if !drain_commands(&state, &receiver, &mut streams) {
            break;
        }
        remove_cancelled_streams(&state, &mut streams);

        let now = Instant::now();
        if now >= next_maintenance {
            if state.maintain_history_rounds().is_err() {
                log::warn!("History maintenance stopped after a runtime interruption");
                break;
            }
            next_maintenance = now + HISTORY_MAINTENANCE_INTERVAL;
        }

        for stream in &mut streams {
            if stream.registration.stop_token().load(Ordering::Acquire)
                || !stream
                    .registration
                    .activation_token()
                    .load(Ordering::Acquire)
                || stream.next_tick > now
            {
                continue;
            }
            if !poll_history_stream(&state, &projection_cache, stream) {
                stream.registration.cancel();
            }
            stream.next_tick = Instant::now() + HISTORY_STREAM_INTERVAL;
        }
        remove_cancelled_streams(&state, &mut streams);

        let wait = streams
            .iter()
            .filter(|stream| {
                stream
                    .registration
                    .activation_token()
                    .load(Ordering::Acquire)
            })
            .map(|stream| stream.next_tick.saturating_duration_since(Instant::now()))
            .chain(std::iter::once(
                next_maintenance.saturating_duration_since(Instant::now()),
            ))
            .min()
            .unwrap_or(HISTORY_RUNTIME_CANCELLATION_SCAN)
            .min(HISTORY_RUNTIME_CANCELLATION_SCAN)
            .max(Duration::from_millis(1));
        match receiver.recv_timeout(wait) {
            Ok(command) => {
                if !handle_command(&state, command, &mut streams) {
                    break;
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }

    stop.store(true, Ordering::Release);
    for stream in streams {
        stream.registration.cancel();
        finish_history_stream(&state, &stream.registration);
    }
}

fn drain_commands(
    state: &AppState,
    receiver: &Receiver<HistoryRuntimeCommand>,
    streams: &mut Vec<HistoryStreamTask>,
) -> bool {
    loop {
        match receiver.try_recv() {
            Ok(command) => {
                if !handle_command(state, command, streams) {
                    return false;
                }
            }
            Err(TryRecvError::Empty) => return true,
            Err(TryRecvError::Disconnected) => return false,
        }
    }
}

fn handle_command(
    state: &AppState,
    command: HistoryRuntimeCommand,
    streams: &mut Vec<HistoryStreamTask>,
) -> bool {
    match command {
        HistoryRuntimeCommand::Register {
            stream,
            acknowledged,
        } => {
            if streams.len() >= MAX_HISTORY_STREAM_TASKS {
                stream.registration.cancel();
                finish_history_stream(state, &stream.registration);
                let _ = acknowledged.send(false);
                return true;
            }
            let task = HistoryStreamTask {
                registration: stream.registration,
                subscription_id: stream.subscription_id,
                on_ready: stream.on_ready,
                last_projection_key: None,
                next_tick: Instant::now(),
            };
            let registration = task.registration.clone();
            streams.push(task);
            if acknowledged.send(true).is_err() {
                registration.cancel();
                remove_cancelled_streams(state, streams);
            }
            true
        }
        HistoryRuntimeCommand::Wake => true,
        HistoryRuntimeCommand::Shutdown => false,
    }
}

fn remove_cancelled_streams(state: &AppState, streams: &mut Vec<HistoryStreamTask>) {
    let mut index = 0;
    while index < streams.len() {
        if streams[index]
            .registration
            .stop_token()
            .load(Ordering::Acquire)
        {
            let stream = streams.swap_remove(index);
            finish_history_stream(state, &stream.registration);
        } else {
            index += 1;
        }
    }
}

fn finish_history_stream(state: &AppState, registration: &StreamRegistration) {
    if state.finish_stream(registration).is_err() {
        log::warn!("History stream registry cleanup reset after an interrupted update");
    }
}

fn poll_history_stream(
    state: &AppState,
    projection_cache: &HistoryProjectionCache,
    stream: &mut HistoryStreamTask,
) -> bool {
    match state.stream_delivery_outstanding(&stream.registration) {
        Ok(true) => return true,
        Ok(false) => {}
        Err(_) => return false,
    }
    let observed_key = history_projection_key(state);
    if stream.last_projection_key == Some(observed_key) {
        return true;
    }
    let reservation = match state.reserve_stream_delivery_capacity(&stream.registration) {
        Ok(Some(reservation)) => reservation,
        Ok(None) => return true,
        Err(_) => return false,
    };
    let projection = match cached_history_stream_projection(state, projection_cache) {
        Ok(Some(projection)) => projection,
        Ok(None) => return true,
        Err(_) => {
            log::warn!("History stream projection cache reset after an interrupted update");
            return true;
        }
    };
    if stream.registration.stop_token().load(Ordering::Acquire) {
        return false;
    }
    if history_projection_key(state) != projection.key {
        return true;
    }
    if stream.last_projection_key == Some(projection.key) {
        return true;
    }
    let sequence = match state.stage_stream_delivery(
        &stream.registration,
        projection.delivery.as_ref().clone(),
        reservation,
    ) {
        Ok(Some(sequence)) => sequence,
        Ok(None) => return true,
        Err(_) => return false,
    };
    if stream.registration.stop_token().load(Ordering::Acquire) {
        return false;
    }
    let signal = StreamReadySignal::new(
        StreamKind::History,
        stream.subscription_id.clone(),
        stream.registration.generation(),
        sequence,
    );
    if stream.on_ready.send(signal).is_err() {
        return false;
    }
    stream.last_projection_key = Some(projection.key);
    true
}

fn history_projection_key(state: &AppState) -> HistoryProjectionKey {
    let resources = state.live_capture_resources();
    HistoryProjectionKey {
        revision: state.history_revision(),
        language: i18n::current_language(),
        character_catalog_identity: Arc::as_ptr(&resources.characters) as usize,
    }
}

fn cached_history_stream_projection(
    state: &AppState,
    cache: &HistoryProjectionCache,
) -> Result<Option<Arc<CachedHistoryProjection>>, HistoryRuntimeError> {
    cached_history_stream_projection_with(state, cache, load_history_summaries)
}

fn cached_history_stream_projection_with(
    state: &AppState,
    cache: &HistoryProjectionCache,
    load: impl FnOnce() -> nte_dps_tool::storage::history::HistoryLoadResult,
) -> Result<Option<Arc<CachedHistoryProjection>>, HistoryRuntimeError> {
    let key = state.with_history_transaction(|| history_projection_key(state))?;
    if let Some(projection) = cache.lookup(key)? {
        return Ok(Some(projection));
    }

    // Directory enumeration and JSON parsing are intentionally outside the
    // mutation transaction. A revision/context change at either boundary makes
    // the projection stale; it is discarded rather than cached or delivered.
    let resources = state.live_capture_resources();
    let characters = resources.characters;
    let loaded = load();
    if state.with_history_transaction(|| history_projection_key(state))? != key {
        return Ok(None);
    }
    let snapshot = HistorySnapshot::from_localized_load_for_language(
        loaded,
        key.revision,
        &characters,
        key.language,
    );
    let delivery = serialize_stream_events(vec![HistoryEvent::Snapshot(snapshot)])
        .map_err(|_| HistoryRuntimeError::Unavailable)?;
    if state.with_history_transaction(|| history_projection_key(state))? != key {
        return Ok(None);
    }
    cache
        .publish(CachedHistoryProjection {
            key,
            delivery: Arc::new(delivery),
        })
        .map(Some)
}

#[cfg(test)]
pub(crate) fn project_history_stream_snapshot(
    state: &AppState,
) -> Result<(u64, HistorySnapshot), HistoryRuntimeError> {
    let key = state.with_history_transaction(|| history_projection_key(state))?;
    let characters = state.live_capture_resources().characters;
    let loaded = load_history_summaries();
    if state.with_history_transaction(|| history_projection_key(state))? != key {
        return Err(HistoryRuntimeError::Unavailable);
    }
    Ok((
        key.revision,
        HistorySnapshot::from_localized_load_for_language(
            loaded,
            key.revision,
            &characters,
            key.language,
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn same_projection_context_reuses_one_loaded_and_serialized_snapshot() {
        let state = AppState::default();
        let cache = HistoryProjectionCache::default();
        let load_count = AtomicUsize::new(0);

        let first = cached_history_stream_projection_with(&state, &cache, || {
            load_count.fetch_add(1, Ordering::SeqCst);
            Default::default()
        })
        .expect("build first History projection")
        .expect("stable first History projection");
        let second = cached_history_stream_projection_with(&state, &cache, || {
            load_count.fetch_add(1, Ordering::SeqCst);
            Default::default()
        })
        .expect("reuse History projection")
        .expect("stable cached History projection");

        assert!(Arc::ptr_eq(&first.delivery, &second.delivery));
        assert_eq!(load_count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn revision_and_language_context_changes_rebuild_the_projection() {
        let cache = HistoryProjectionCache::default();
        let builds = AtomicUsize::new(0);
        let base = HistoryProjectionKey::fixture(7, Language::English, 11);

        cache
            .get_or_build(base, || Ok(counted_fixture_projection(&builds, base)))
            .expect("build base projection");
        cache
            .get_or_build(
                HistoryProjectionKey::fixture(8, Language::English, 11),
                || {
                    Ok(counted_fixture_projection(
                        &builds,
                        HistoryProjectionKey::fixture(8, Language::English, 11),
                    ))
                },
            )
            .expect("rebuild after revision change");
        cache
            .get_or_build(
                HistoryProjectionKey::fixture(8, Language::Japanese, 11),
                || {
                    Ok(counted_fixture_projection(
                        &builds,
                        HistoryProjectionKey::fixture(8, Language::Japanese, 11),
                    ))
                },
            )
            .expect("rebuild after language change");

        assert_eq!(builds.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn poisoned_projection_cache_fails_one_lookup_then_rebuilds_cleanly() {
        let cache = HistoryProjectionCache::default();
        cache.poison_for_test();
        let key = HistoryProjectionKey::fixture(1, Language::English, 1);

        assert!(
            cache
                .get_or_build(key, || Ok(fixture_projection(key)))
                .is_err()
        );
        assert!(
            cache
                .get_or_build(key, || Ok(fixture_projection(key)))
                .is_ok()
        );
        assert_eq!(cache.entry_count_for_test(), 1);
    }

    fn counted_fixture_projection(
        builds: &AtomicUsize,
        key: HistoryProjectionKey,
    ) -> CachedHistoryProjection {
        builds.fetch_add(1, Ordering::SeqCst);
        fixture_projection(key)
    }

    fn fixture_projection(key: HistoryProjectionKey) -> CachedHistoryProjection {
        CachedHistoryProjection {
            key,
            delivery: Arc::new(vec![b'{', b'}']),
        }
    }

    #[test]
    fn projected_snapshot_carries_the_stable_revision_read_around_the_scan() {
        let state = AppState::default();
        state.bump_history_revision();

        let (revision, snapshot) =
            project_history_stream_snapshot(&state).expect("project History stream snapshot");

        assert_eq!(snapshot.revision, revision.to_string());
    }

    #[test]
    fn revision_change_during_summary_scan_discards_projection_before_cache_publish() {
        let state = AppState::default();
        let cache = HistoryProjectionCache::default();

        let projection = cached_history_stream_projection_with(&state, &cache, || {
            state
                .with_history_transaction(|| state.bump_history_revision())
                .expect("mutate History during summary scan");
            Default::default()
        })
        .expect("stale scan is not a runtime failure");

        assert!(projection.is_none());
        assert_eq!(cache.entry_count_for_test(), 0);
    }

    #[test]
    fn slow_summary_scan_does_not_hold_history_mutation_transaction() {
        use std::sync::mpsc;

        let state = AppState::default();
        let cache = Arc::new(HistoryProjectionCache::default());
        let (load_started_tx, load_started_rx) = mpsc::channel();
        let (release_load_tx, release_load_rx) = mpsc::channel();
        let worker_state = state.clone();
        let worker_cache = Arc::clone(&cache);
        let worker = thread::spawn(move || {
            cached_history_stream_projection_with(&worker_state, &worker_cache, || {
                load_started_tx.send(()).expect("signal summary scan start");
                release_load_rx.recv().expect("release summary scan");
                Default::default()
            })
        });
        load_started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("summary scan started");

        let (transaction_done_tx, transaction_done_rx) = mpsc::channel();
        let transaction_state = state.clone();
        let transaction = thread::spawn(move || {
            transaction_state
                .with_history_transaction(|| ())
                .expect("short History mutation transaction");
            transaction_done_tx.send(()).expect("signal transaction");
        });
        let transaction_completed = transaction_done_rx
            .recv_timeout(Duration::from_millis(250))
            .is_ok();
        release_load_tx.send(()).expect("finish summary scan");
        transaction.join().expect("join transaction probe");
        worker
            .join()
            .expect("join summary scan probe")
            .expect("summary scan runtime")
            .expect(
                "summary projection remains valid when an empty transaction does not bump revision",
            );

        assert!(transaction_completed);
    }

    #[test]
    fn shutdown_is_idempotent_and_joins_the_owned_worker() {
        let runtime = HistoryRuntime::start(AppState::default()).expect("start History runtime");
        let key = HistoryProjectionKey::fixture(1, Language::English, 1);
        runtime
            .projection_cache
            .publish(fixture_projection(key))
            .expect("seed History projection cache");
        assert_eq!(runtime.projection_cache.entry_count_for_test(), 1);

        assert!(runtime.shutdown());
        assert_eq!(runtime.projection_cache.entry_count_for_test(), 0);
        assert!(!runtime.shutdown());
    }

    #[test]
    fn cancelled_stream_is_woken_cleaned_and_owned_until_shutdown() {
        let state = AppState::default();
        let runtime = HistoryRuntime::start(state.clone()).expect("start History runtime");
        let stream_kind = StreamKind::History;
        let registration = state
            .reserve_stream("console", &stream_kind.stream_key("owned"))
            .expect("reserve History stream");
        runtime
            .register_stream(HistoryStreamRegistration {
                registration: registration.clone(),
                subscription_id: "owned".to_owned(),
                on_ready: Channel::new(|_| Ok(())),
            })
            .expect("register owned History stream");

        registration.cancel();
        runtime.wake();
        let deadline = Instant::now() + Duration::from_secs(1);
        while state.stream_registry_len() != Ok(0) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(state.stream_registry_len(), Ok(0));
        assert!(runtime.shutdown());
    }
}
