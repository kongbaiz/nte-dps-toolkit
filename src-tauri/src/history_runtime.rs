use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use nte_dps_tool::storage::history::load_history_summaries;
use nte_dps_tool::storage::i18n::{self, Language};
use tauri::{async_runtime::JoinHandle, ipc::Channel};
use tokio::time::{MissedTickBehavior, interval};

use crate::{
    contract::{
        CommandError,
        history::{HistoryEvent, HistorySnapshot},
        stream::{MAX_STREAM_DELIVERY_BYTES, StreamDeliveryBody},
    },
    state::{AppState, HistoryRuntimeError, StreamRegistration},
};

const HISTORY_MAINTENANCE_INTERVAL: Duration = Duration::from_millis(250);
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
    state: AppState,
    stop: Arc<AtomicBool>,
    maintenance: Mutex<Option<JoinHandle<()>>>,
    projection_cache: Arc<HistoryProjectionCache>,
}

pub(crate) struct HistoryStreamRegistration {
    pub(crate) registration: StreamRegistration,
    pub(crate) on_event: Channel<StreamDeliveryBody<HistoryEvent>>,
}

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
    snapshot: Arc<HistorySnapshot>,
}

impl HistoryProjectionCache {
    fn lookup(
        &self,
        key: HistoryProjectionKey,
    ) -> Result<Option<Arc<CachedHistoryProjection>>, HistoryRuntimeError> {
        let entry = self.entry.lock().map_err(|mut poison| {
            **poison.get_mut() = None;
            self.entry.clear_poison();
            HistoryRuntimeError::Unavailable
        })?;
        Ok(entry.as_ref().filter(|entry| entry.key == key).cloned())
    }

    fn publish(
        &self,
        projection: CachedHistoryProjection,
    ) -> Result<Arc<CachedHistoryProjection>, HistoryRuntimeError> {
        let mut entry = self.entry.lock().map_err(|mut poison| {
            **poison.get_mut() = None;
            self.entry.clear_poison();
            HistoryRuntimeError::Unavailable
        })?;
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
    fn entry_count_for_test(&self) -> usize {
        self.entry
            .lock()
            .map(|entry| entry.iter().count())
            .unwrap_or(0)
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
    pub(crate) fn start(state: AppState) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let task_state = state.clone();
        let task_stop = Arc::clone(&stop);
        let maintenance = tauri::async_runtime::spawn(async move {
            let mut ticker = interval(HISTORY_MAINTENANCE_INTERVAL);
            ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
            while !task_stop.load(Ordering::Acquire) {
                ticker.tick().await;
                if task_stop.load(Ordering::Acquire) {
                    break;
                }
                let state = task_state.clone();
                match tauri::async_runtime::spawn_blocking(move || state.maintain_history_rounds())
                    .await
                {
                    Ok(Ok(())) => {}
                    Ok(Err(_)) | Err(_) => {
                        log::warn!("History maintenance stopped after a runtime interruption");
                        task_stop.store(true, Ordering::Release);
                    }
                }
            }
        });
        Self {
            state,
            stop,
            maintenance: Mutex::new(Some(maintenance)),
            projection_cache: Arc::new(HistoryProjectionCache::default()),
        }
    }

    pub(crate) fn register_stream(&self, stream: HistoryStreamRegistration) -> Result<(), ()> {
        if self.stop.load(Ordering::Acquire) {
            return Err(());
        }
        match self.state.activate_stream(&stream.registration) {
            Ok(true) => {}
            Ok(false) | Err(_) => return Err(()),
        }

        let state = self.state.clone();
        let runtime_stop = Arc::clone(&self.stop);
        let cache = Arc::clone(&self.projection_cache);
        tauri::async_runtime::spawn(async move {
            run_history_stream(state, runtime_stop, cache, stream).await;
        });
        Ok(())
    }

    pub(crate) fn shutdown(&self) -> bool {
        self.stop.store(true, Ordering::Release);
        let maintenance = match self.maintenance.lock() {
            Ok(mut task) => task.take(),
            Err(mut poison) => {
                let task = poison.get_mut().take();
                self.maintenance.clear_poison();
                task
            }
        };
        if let Some(task) = &maintenance {
            task.abort();
        }
        self.projection_cache.clear();
        maintenance.is_some()
    }
}

impl Drop for HistoryRuntime {
    fn drop(&mut self) {
        self.shutdown();
    }
}

async fn run_history_stream(
    state: AppState,
    runtime_stop: Arc<AtomicBool>,
    cache: Arc<HistoryProjectionCache>,
    stream: HistoryStreamRegistration,
) {
    let stop = stream.registration.stop_token();
    let mut last_projection_key = None;
    let mut ticker = interval(HISTORY_STREAM_INTERVAL);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

    while !runtime_stop.load(Ordering::Acquire) && !stop.load(Ordering::Acquire) {
        ticker.tick().await;
        if runtime_stop.load(Ordering::Acquire) || stop.load(Ordering::Acquire) {
            break;
        }
        let observed_key = history_projection_key(&state);
        if last_projection_key == Some(observed_key) {
            continue;
        }

        let projection_state = state.clone();
        let projection_cache = Arc::clone(&cache);
        let projection = tauri::async_runtime::spawn_blocking(move || {
            cached_history_stream_projection(&projection_state, &projection_cache)
        })
        .await;
        let projection = match projection {
            Ok(Ok(Some(projection))) => projection,
            Ok(Ok(None)) => continue,
            Ok(Err(_)) | Err(_) => break,
        };
        if stop.load(Ordering::Acquire) || history_projection_key(&state) != projection.key {
            continue;
        }

        let body = StreamDeliveryBody::new(vec![HistoryEvent::Snapshot(
            projection.snapshot.as_ref().clone(),
        )]);
        if serde_json::to_vec(&body)
            .map(|encoded| encoded.len() > MAX_STREAM_DELIVERY_BYTES)
            .unwrap_or(true)
            || stream.on_event.send(body).is_err()
        {
            break;
        }
        last_projection_key = Some(projection.key);
    }

    stream.registration.cancel();
    if state.finish_stream(&stream.registration).is_err() {
        log::warn!("History stream registry cleanup reset after an interrupted update");
    }
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

    let characters = state.live_capture_resources().characters;
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
    if state.with_history_transaction(|| history_projection_key(state))? != key {
        return Ok(None);
    }
    cache
        .publish(CachedHistoryProjection {
            key,
            snapshot: Arc::new(snapshot),
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
    use std::{sync::atomic::AtomicUsize, thread};

    #[test]
    fn same_projection_context_reuses_one_snapshot() {
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

        assert!(Arc::ptr_eq(&first.snapshot, &second.snapshot));
        assert_eq!(load_count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn revision_and_language_context_changes_rebuild_the_projection() {
        let cache = HistoryProjectionCache::default();
        let builds = AtomicUsize::new(0);
        let keys = [
            HistoryProjectionKey::fixture(7, Language::English, 11),
            HistoryProjectionKey::fixture(8, Language::English, 11),
            HistoryProjectionKey::fixture(8, Language::Japanese, 11),
        ];
        for key in keys {
            cache
                .get_or_build(key, || {
                    builds.fetch_add(1, Ordering::SeqCst);
                    Ok(fixture_projection(key))
                })
                .expect("build projection");
        }
        assert_eq!(builds.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn revision_change_during_summary_scan_discards_projection() {
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
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let worker_state = state.clone();
        let worker_cache = Arc::clone(&cache);
        let worker = thread::spawn(move || {
            cached_history_stream_projection_with(&worker_state, &worker_cache, || {
                started_tx.send(()).expect("signal scan start");
                release_rx.recv().expect("release scan");
                Default::default()
            })
        });
        started_rx.recv().expect("summary scan started");
        state
            .with_history_transaction(|| ())
            .expect("short History mutation transaction");
        release_tx.send(()).expect("finish summary scan");
        worker.join().expect("join scan").expect("projection");
    }

    fn fixture_projection(key: HistoryProjectionKey) -> CachedHistoryProjection {
        CachedHistoryProjection {
            key,
            snapshot: Arc::new(HistorySnapshot::from_load(Default::default(), key.revision)),
        }
    }
}
