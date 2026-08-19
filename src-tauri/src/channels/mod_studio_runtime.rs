use std::{
    io,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc, Mutex, MutexGuard, OnceLock,
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError, sync_channel},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use nte_dps_tool::{
    core::mod_studio::ModStudioRuntimeSnapshot, platform::mods_plugin::probe_runtime_presence,
};

use crate::state::AppState;

const MOD_STUDIO_POLL_QUEUE_CAPACITY: usize = 1;
const MOD_STUDIO_POLL_WAKE_INTERVAL: Duration = Duration::from_millis(25);

static MOD_STUDIO_POLL_RUNTIME: OnceLock<Mutex<Option<ModStudioPollRuntime>>> = OnceLock::new();
static MOD_STUDIO_POLL_START: Mutex<()> = Mutex::new(());

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ModStudioPollState {
    Connected(Arc<ModStudioRuntimeSnapshot>),
    LoaderPresent,
    Waiting,
    ProbeFailed,
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

/// Returns the newest completed poll and non-blockingly requests one fresh poll.
///
/// Capacity and backpressure contract:
/// - producer: one fixed `nte-mod-studio-poll` thread;
/// - capacity: one queued or in-flight request for the entire application;
/// - ordering: completed observations use a monotonic generation;
/// - full: concurrent requests coalesce into the existing request;
/// - disconnect/panic: fail closed without exposing native error details;
/// - shutdown: stop accepting work, wake, and join outside the runtime slot lock.
pub(crate) fn observe_and_request_mod_studio_poll(
    state: &AppState,
) -> Result<Option<ModStudioPollObservation>, ModStudioPollRuntimeError> {
    let handle = mod_studio_poll_runtime_handle()?;
    let observation = handle.observe()?;
    handle.request(state.clone())?;
    Ok(observation)
}

pub(crate) fn shutdown_mod_studio_poll_runtime() -> bool {
    let Some(slot) = MOD_STUDIO_POLL_RUNTIME.get() else {
        return false;
    };
    let runtime = match slot.lock() {
        Ok(mut runtime) => runtime.take(),
        Err(mut poison) => {
            let runtime = poison.get_mut().take();
            slot.clear_poison();
            drop(poison);
            runtime
        }
    };
    let stopped = runtime.is_some();
    drop(runtime);
    stopped
}

fn mod_studio_poll_runtime_handle() -> Result<ModStudioPollHandle, ModStudioPollRuntimeError> {
    let _start_guard = match MOD_STUDIO_POLL_START.lock() {
        Ok(guard) => guard,
        Err(poison) => {
            drop(poison);
            MOD_STUDIO_POLL_START.clear_poison();
            return Err(ModStudioPollRuntimeError::Unavailable);
        }
    };
    let slot = MOD_STUDIO_POLL_RUNTIME.get_or_init(|| Mutex::new(None));
    loop {
        let stale = {
            let mut runtime = lock_runtime_slot(slot)?;
            if let Some(existing) = runtime.as_ref()
                && existing.is_healthy()
            {
                return Ok(existing.handle());
            }
            runtime.take()
        };
        drop(stale);

        let started = ModStudioPollRuntime::start_with(
            Arc::new(poll_mod_studio_runtime),
            &SystemThreadSpawner,
        )
        .map_err(|_| ModStudioPollRuntimeError::Unavailable)?;
        let handle = started.handle();
        let mut runtime = lock_runtime_slot(slot)?;
        if runtime.is_none() {
            *runtime = Some(started);
            return Ok(handle);
        }
        drop(runtime);
        drop(started);
    }
}

fn lock_runtime_slot(
    slot: &Mutex<Option<ModStudioPollRuntime>>,
) -> Result<MutexGuard<'_, Option<ModStudioPollRuntime>>, ModStudioPollRuntimeError> {
    match slot.lock() {
        Ok(runtime) => Ok(runtime),
        Err(mut poison) => {
            let stale = poison.get_mut().take();
            slot.clear_poison();
            drop(poison);
            drop(stale);
            Err(ModStudioPollRuntimeError::Unavailable)
        }
    }
}

fn poll_mod_studio_runtime(state: &AppState) -> ModStudioPollState {
    match state.poll_mod_studio_runtime() {
        Ok(snapshot) => ModStudioPollState::Connected(Arc::new(snapshot)),
        Err(_) => match probe_runtime_presence() {
            Ok(true) => ModStudioPollState::LoaderPresent,
            Ok(false) => ModStudioPollState::Waiting,
            Err(_) => ModStudioPollState::ProbeFailed,
        },
    }
}

#[derive(Default)]
struct ModStudioPollCache {
    generation: u64,
    latest: Option<ModStudioPollState>,
}

struct ModStudioPollShared {
    busy: AtomicBool,
    healthy: AtomicBool,
    shutdown: AtomicBool,
    cache: Mutex<ModStudioPollCache>,
}

impl Default for ModStudioPollShared {
    fn default() -> Self {
        Self {
            busy: AtomicBool::new(false),
            healthy: AtomicBool::new(true),
            shutdown: AtomicBool::new(false),
            cache: Mutex::new(ModStudioPollCache::default()),
        }
    }
}

enum ModStudioPollCommand {
    Poll(AppState),
    Shutdown,
}

type PollOperation = Arc<dyn Fn(&AppState) -> ModStudioPollState + Send + Sync>;

struct ModStudioPollRuntime {
    sender: SyncSender<ModStudioPollCommand>,
    shared: Arc<ModStudioPollShared>,
    worker: Option<JoinHandle<()>>,
}

#[derive(Clone)]
struct ModStudioPollHandle {
    sender: SyncSender<ModStudioPollCommand>,
    shared: Arc<ModStudioPollShared>,
}

impl ModStudioPollRuntime {
    fn start_with(operation: PollOperation, spawner: &dyn ThreadSpawner) -> io::Result<Self> {
        let (sender, receiver) = sync_channel(MOD_STUDIO_POLL_QUEUE_CAPACITY);
        let shared = Arc::new(ModStudioPollShared::default());
        let worker_shared = Arc::clone(&shared);
        let worker = spawner.spawn(
            "nte-mod-studio-poll".to_owned(),
            Box::new(move || run_mod_studio_poll_worker(receiver, worker_shared, operation)),
        )?;
        Ok(Self {
            sender,
            shared,
            worker: Some(worker),
        })
    }

    fn handle(&self) -> ModStudioPollHandle {
        ModStudioPollHandle {
            sender: self.sender.clone(),
            shared: Arc::clone(&self.shared),
        }
    }

    fn is_healthy(&self) -> bool {
        self.shared.healthy.load(Ordering::Acquire) && !self.shared.shutdown.load(Ordering::Acquire)
    }
}

impl Drop for ModStudioPollRuntime {
    fn drop(&mut self) {
        self.shared.shutdown.store(true, Ordering::Release);
        self.shared.healthy.store(false, Ordering::Release);
        let _ = self.sender.try_send(ModStudioPollCommand::Shutdown);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl ModStudioPollHandle {
    fn observe(&self) -> Result<Option<ModStudioPollObservation>, ModStudioPollRuntimeError> {
        let cache = match self.shared.cache.lock() {
            Ok(cache) => cache,
            Err(mut poison) => {
                **poison.get_mut() = ModStudioPollCache::default();
                self.shared.cache.clear_poison();
                self.shared.healthy.store(false, Ordering::Release);
                return Err(ModStudioPollRuntimeError::Unavailable);
            }
        };
        Ok(cache.latest.clone().map(|state| ModStudioPollObservation {
            generation: cache.generation,
            state,
        }))
    }

    fn request(&self, state: AppState) -> Result<(), ModStudioPollRuntimeError> {
        if !self.shared.healthy.load(Ordering::Acquire)
            || self.shared.shutdown.load(Ordering::Acquire)
        {
            return Err(ModStudioPollRuntimeError::Unavailable);
        }
        if self
            .shared
            .busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Ok(());
        }
        match self.sender.try_send(ModStudioPollCommand::Poll(state)) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => {
                self.shared.busy.store(false, Ordering::Release);
                Ok(())
            }
            Err(TrySendError::Disconnected(_)) => {
                self.shared.busy.store(false, Ordering::Release);
                self.shared.healthy.store(false, Ordering::Release);
                Err(ModStudioPollRuntimeError::Unavailable)
            }
        }
    }
}

fn run_mod_studio_poll_worker(
    receiver: Receiver<ModStudioPollCommand>,
    shared: Arc<ModStudioPollShared>,
    operation: PollOperation,
) {
    while !shared.shutdown.load(Ordering::Acquire) {
        let state = match receiver.recv_timeout(MOD_STUDIO_POLL_WAKE_INTERVAL) {
            Ok(ModStudioPollCommand::Poll(state)) => state,
            Ok(ModStudioPollCommand::Shutdown) => break,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => break,
        };
        if shared.shutdown.load(Ordering::Acquire) {
            shared.busy.store(false, Ordering::Release);
            break;
        }
        let next = catch_unwind(AssertUnwindSafe(|| operation(&state)))
            .unwrap_or(ModStudioPollState::ProbeFailed);
        if !shared.shutdown.load(Ordering::Acquire) {
            if let Ok(mut cache) = shared.cache.lock() {
                cache.generation = cache.generation.saturating_add(1).max(1);
                cache.latest = Some(next);
            } else {
                shared.healthy.store(false, Ordering::Release);
            }
        }
        shared.busy.store(false, Ordering::Release);
    }
    shared.busy.store(false, Ordering::Release);
    if !shared.shutdown.load(Ordering::Acquire) {
        shared.healthy.store(false, Ordering::Release);
    }
}

trait ThreadSpawner {
    fn spawn(
        &self,
        name: String,
        job: Box<dyn FnOnce() + Send + 'static>,
    ) -> io::Result<JoinHandle<()>>;
}

struct SystemThreadSpawner;

impl ThreadSpawner for SystemThreadSpawner {
    fn spawn(
        &self,
        name: String,
        job: Box<dyn FnOnce() + Send + 'static>,
    ) -> io::Result<JoinHandle<()>> {
        thread::Builder::new().name(name).spawn(job)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Barrier, atomic::AtomicUsize, mpsc};

    use super::*;

    #[test]
    fn blocked_poll_coalesces_sibling_requests_without_blocking_callers() {
        let entered = Arc::new(Barrier::new(2));
        let (release, released) = mpsc::channel();
        let released = Arc::new(Mutex::new(released));
        let calls = Arc::new(AtomicUsize::new(0));
        let runtime = ModStudioPollRuntime::start_with(
            {
                let entered = Arc::clone(&entered);
                let released = Arc::clone(&released);
                let calls = Arc::clone(&calls);
                Arc::new(move |_| {
                    calls.fetch_add(1, Ordering::AcqRel);
                    entered.wait();
                    released
                        .lock()
                        .expect("release receiver lock")
                        .recv()
                        .expect("release blocked poll");
                    ModStudioPollState::Waiting
                })
            },
            &SystemThreadSpawner,
        )
        .expect("start test Mod Studio lane");
        let handle = runtime.handle();
        handle
            .request(AppState::default())
            .expect("queue first poll");
        entered.wait();

        for _ in 0..16 {
            handle
                .request(AppState::default())
                .expect("coalesce sibling poll");
        }
        assert_eq!(calls.load(Ordering::Acquire), 1);

        release.send(()).expect("release poll");
        drop(runtime);
    }

    #[test]
    fn panicking_poll_publishes_probe_failed_and_worker_accepts_the_next_poll() {
        let calls = Arc::new(AtomicUsize::new(0));
        let runtime = ModStudioPollRuntime::start_with(
            {
                let calls = Arc::clone(&calls);
                Arc::new(move |_| {
                    if calls.fetch_add(1, Ordering::AcqRel) == 0 {
                        panic!("fixture poll panic");
                    }
                    ModStudioPollState::LoaderPresent
                })
            },
            &SystemThreadSpawner,
        )
        .expect("start test Mod Studio lane");
        let handle = runtime.handle();
        handle
            .request(AppState::default())
            .expect("queue panic poll");
        let first = wait_for_generation(&handle, 1);
        assert_eq!(first.state, ModStudioPollState::ProbeFailed);

        handle
            .request(AppState::default())
            .expect("queue recovery poll");
        let second = wait_for_generation(&handle, 2);
        assert_eq!(second.state, ModStudioPollState::LoaderPresent);
        assert_eq!(calls.load(Ordering::Acquire), 2);

        drop(runtime);
    }

    fn wait_for_generation(
        handle: &ModStudioPollHandle,
        expected: u64,
    ) -> ModStudioPollObservation {
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        loop {
            if let Some(observation) = handle.observe().expect("observe poll cache")
                && observation.generation >= expected
            {
                return observation;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "poll result timed out"
            );
            thread::sleep(Duration::from_millis(1));
        }
    }
}
