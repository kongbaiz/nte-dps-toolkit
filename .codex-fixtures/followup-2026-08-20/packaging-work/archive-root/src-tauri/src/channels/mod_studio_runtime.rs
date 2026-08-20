use std::{
    io,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc, Mutex, MutexGuard, OnceLock,
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError, sync_channel},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use nte_dps_tool::{
    core::mod_studio::ModStudioRuntimeSnapshot,
    platform::{
        mods_plugin::probe_runtime_presence,
        mods_plugin_bootstrap::{
            ModsPluginBootstrapError, ModsPluginBootstrapErrorCode, ModsPluginInitializeOutcome,
            RunningModsPluginBootstrapContext, initialize_running_deployed_mods_plugin_context,
            inspect_running_deployed_mods_plugin_context,
        },
    },
    storage::config::ModStudioLoadingMethod,
};

use crate::state::AppState;

const MOD_STUDIO_POLL_QUEUE_CAPACITY: usize = 1;
const MOD_STUDIO_POLL_WAKE_INTERVAL: Duration = Duration::from_millis(25);
const MOD_STUDIO_OWNER_POLL_INTERVAL: Duration = Duration::from_secs(1);
const MOD_STUDIO_BOOTSTRAP_RETRY_BASE: Duration = Duration::from_secs(1);
const MOD_STUDIO_BOOTSTRAP_RETRY_MAX: Duration = Duration::from_secs(8);

static MOD_STUDIO_POLL_RUNTIME: OnceLock<Mutex<Option<ModStudioPollRuntime>>> = OnceLock::new();
static MOD_STUDIO_POLL_START: Mutex<()> = Mutex::new(());

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ModStudioPollState {
    Connected(Arc<ModStudioRuntimeSnapshot>),
    LoaderPresent,
    Waiting,
    AcknowledgementRequired,
    ProbeFailed,
    BootstrapFailed(ModsPluginBootstrapErrorCode),
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

/// Application-owned driver for proxy bootstrap and runtime-health polling.
///
/// The page subscription only consumes the shared observation cache; this
/// owner keeps the loader-lock-free bootstrap alive even when Mod Studio is
/// hidden. The driver has one worker, a one-slot wake queue, coalesced ticks,
/// and explicit application-shutdown cancellation. Native polling remains on
/// the separate bounded worker above and never runs under an AppState lock.
pub(crate) struct ModStudioMonitorRuntime {
    stop: Arc<AtomicBool>,
    wake: SyncSender<()>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl ModStudioMonitorRuntime {
    pub(crate) fn start(state: AppState) -> io::Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let (wake, receiver) = sync_channel(1);
        let worker_stop = Arc::clone(&stop);
        let worker = thread::Builder::new()
            .name("nte-mod-studio-monitor".to_owned())
            .spawn(move || {
                run_mod_studio_monitor(
                    worker_stop.as_ref(),
                    receiver,
                    MOD_STUDIO_OWNER_POLL_INTERVAL,
                    || {
                        let _ = observe_and_request_mod_studio_poll(&state);
                    },
                );
            })?;
        Ok(Self {
            stop,
            wake,
            worker: Mutex::new(Some(worker)),
        })
    }

    pub(crate) fn unavailable() -> Self {
        let (wake, receiver) = sync_channel(1);
        drop(receiver);
        Self {
            stop: Arc::new(AtomicBool::new(true)),
            wake,
            worker: Mutex::new(None),
        }
    }

    /// Idempotently cancels and joins the owned scheduler outside its mutex.
    pub(crate) fn shutdown(&self) -> bool {
        self.stop.store(true, Ordering::Release);
        let _ = self.wake.try_send(());
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
        stopped
    }
}

impl Drop for ModStudioMonitorRuntime {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn run_mod_studio_monitor(
    stop: &AtomicBool,
    receiver: Receiver<()>,
    interval: Duration,
    mut request: impl FnMut(),
) {
    while !stop.load(Ordering::Acquire) {
        request();
        match receiver.recv_timeout(interval) {
            Ok(()) | Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
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

        let started =
            ModStudioPollRuntime::start_with(system_poll_operation(), &SystemThreadSpawner)
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

fn system_poll_operation() -> PollOperation {
    let retry = Mutex::new(BootstrapRetryPolicy::new(
        MOD_STUDIO_BOOTSTRAP_RETRY_BASE,
        MOD_STUDIO_BOOTSTRAP_RETRY_MAX,
    ));
    Arc::new(move |state| {
        let mut retry = match retry.lock() {
            Ok(retry) => retry,
            Err(mut poison) => {
                **poison.get_mut() = BootstrapRetryPolicy::new(
                    MOD_STUDIO_BOOTSTRAP_RETRY_BASE,
                    MOD_STUDIO_BOOTSTRAP_RETRY_MAX,
                );
                retry.clear_poison();
                return ModStudioPollState::ProbeFailed;
            }
        };
        poll_mod_studio_runtime(state, &mut retry, Instant::now())
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ModStudioBootstrapContext {
    settings_generation: u64,
    loading_method: ModStudioLoadingMethod,
    risk_acknowledged: bool,
    target: ModStudioBootstrapTarget,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ModStudioBootstrapTarget {
    NoGame,
    Available(RunningModsPluginBootstrapContext),
    InspectionFailed(ModsPluginBootstrapErrorCode),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BootstrapRetryDisposition {
    Success,
    Recoverable,
    StableFailure,
}

struct BootstrapAttempt {
    state: ModStudioPollState,
    disposition: BootstrapRetryDisposition,
}

struct BootstrapRetryPolicy<C> {
    context: Option<C>,
    latest: Option<ModStudioPollState>,
    stable_failure: bool,
    recoverable_attempts: u32,
    next_attempt: Option<Instant>,
    base_delay: Duration,
    max_delay: Duration,
}

impl<C: PartialEq> BootstrapRetryPolicy<C> {
    fn new(base_delay: Duration, max_delay: Duration) -> Self {
        debug_assert!(!base_delay.is_zero());
        debug_assert!(max_delay >= base_delay);
        Self {
            context: None,
            latest: None,
            stable_failure: false,
            recoverable_attempts: 0,
            next_attempt: None,
            base_delay,
            max_delay,
        }
    }

    fn reset(&mut self) {
        self.context = None;
        self.latest = None;
        self.stable_failure = false;
        self.recoverable_attempts = 0;
        self.next_attempt = None;
    }

    fn evaluate(
        &mut self,
        context: C,
        now: Instant,
        operation: impl FnOnce() -> BootstrapAttempt,
    ) -> ModStudioPollState {
        if self.context.as_ref() != Some(&context) {
            self.context = Some(context);
            self.latest = None;
            self.stable_failure = false;
            self.recoverable_attempts = 0;
            self.next_attempt = None;
        }
        if self.stable_failure
            || self
                .next_attempt
                .is_some_and(|next_attempt| now < next_attempt)
        {
            return self.latest.clone().unwrap_or(ModStudioPollState::Waiting);
        }

        let attempt = operation();
        self.latest = Some(attempt.state.clone());
        match attempt.disposition {
            BootstrapRetryDisposition::Success => {
                self.stable_failure = false;
                self.recoverable_attempts = 0;
                self.next_attempt = None;
            }
            BootstrapRetryDisposition::StableFailure => {
                self.stable_failure = true;
                self.recoverable_attempts = 0;
                self.next_attempt = None;
            }
            BootstrapRetryDisposition::Recoverable => {
                let exponent = self.recoverable_attempts.min(31);
                let delay = self
                    .base_delay
                    .saturating_mul(1_u32 << exponent)
                    .min(self.max_delay);
                self.recoverable_attempts = self.recoverable_attempts.saturating_add(1);
                self.next_attempt = now.checked_add(delay);
            }
        }
        attempt.state
    }
}

fn poll_mod_studio_runtime(
    state: &AppState,
    retry: &mut BootstrapRetryPolicy<ModStudioBootstrapContext>,
    now: Instant,
) -> ModStudioPollState {
    match state.poll_mod_studio_runtime() {
        Ok(snapshot) => {
            retry.reset();
            ModStudioPollState::Connected(Arc::new(snapshot))
        }
        Err(_) => match probe_runtime_presence() {
            Ok(true) => {
                retry.reset();
                ModStudioPollState::LoaderPresent
            }
            Ok(false) => {
                let loading_method = state.mod_studio_loading_method();
                let risk_acknowledged = state.mod_studio_risk_acknowledged();
                match mod_studio_start_precondition(loading_method, risk_acknowledged) {
                    Some(blocked) => {
                        retry.reset();
                        blocked
                    }
                    None => {
                        let target = match inspect_running_deployed_mods_plugin_context() {
                            Ok(Some(context)) => ModStudioBootstrapTarget::Available(context),
                            Ok(None) => ModStudioBootstrapTarget::NoGame,
                            Err(error) => ModStudioBootstrapTarget::InspectionFailed(error.code()),
                        };
                        let context = ModStudioBootstrapContext {
                            settings_generation: state.settings_revision(),
                            loading_method,
                            risk_acknowledged,
                            target: target.clone(),
                        };
                        retry.evaluate(context, now, || match target {
                            ModStudioBootstrapTarget::NoGame => BootstrapAttempt {
                                state: ModStudioPollState::Waiting,
                                disposition: BootstrapRetryDisposition::Recoverable,
                            },
                            ModStudioBootstrapTarget::InspectionFailed(_) => BootstrapAttempt {
                                state: ModStudioPollState::ProbeFailed,
                                disposition: BootstrapRetryDisposition::Recoverable,
                            },
                            ModStudioBootstrapTarget::Available(context) => {
                                classify_proxy_bootstrap_attempt(
                                    initialize_running_deployed_mods_plugin_context(&context),
                                )
                            }
                        })
                    }
                }
            }
            Err(_) => ModStudioPollState::ProbeFailed,
        },
    }
}

fn classify_proxy_bootstrap_attempt(
    result: Result<ModsPluginInitializeOutcome, ModsPluginBootstrapError>,
) -> BootstrapAttempt {
    match result {
        Ok(ModsPluginInitializeOutcome::Started | ModsPluginInitializeOutcome::AlreadyRunning) => {
            BootstrapAttempt {
                state: ModStudioPollState::LoaderPresent,
                disposition: BootstrapRetryDisposition::Success,
            }
        }
        Err(
            ModsPluginBootstrapError::DeployedModuleNotLoaded
            | ModsPluginBootstrapError::PluginUnavailable(io::ErrorKind::NotFound)
            | ModsPluginBootstrapError::LocalBootstrapInProgress
            | ModsPluginBootstrapError::InitializationInProgress
            | ModsPluginBootstrapError::ProcessIdentityChanged,
        ) => BootstrapAttempt {
            state: ModStudioPollState::Waiting,
            disposition: BootstrapRetryDisposition::Recoverable,
        },
        Err(error) => BootstrapAttempt {
            state: ModStudioPollState::BootstrapFailed(error.code()),
            disposition: BootstrapRetryDisposition::StableFailure,
        },
    }
}

fn mod_studio_start_precondition(
    method: ModStudioLoadingMethod,
    risk_acknowledged: bool,
) -> Option<ModStudioPollState> {
    if !risk_acknowledged {
        Some(ModStudioPollState::AcknowledgementRequired)
    } else if method == ModStudioLoadingMethod::Loader {
        Some(ModStudioPollState::Waiting)
    } else {
        None
    }
}

#[cfg(test)]
fn classify_proxy_bootstrap(
    result: Result<Option<ModsPluginInitializeOutcome>, ModsPluginBootstrapError>,
) -> ModStudioPollState {
    match result {
        Ok(Some(
            ModsPluginInitializeOutcome::Started | ModsPluginInitializeOutcome::AlreadyRunning,
        )) => ModStudioPollState::LoaderPresent,
        Ok(None)
        | Err(ModsPluginBootstrapError::DeployedModuleNotLoaded)
        | Err(ModsPluginBootstrapError::PluginUnavailable(io::ErrorKind::NotFound))
        | Err(ModsPluginBootstrapError::LocalBootstrapInProgress) => ModStudioPollState::Waiting,
        Err(error) => ModStudioPollState::BootstrapFailed(error.code()),
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
    fn application_owned_monitor_polls_without_a_page_and_stops_on_owner_shutdown() {
        let stop = Arc::new(AtomicBool::new(false));
        let (wake, receiver) = sync_channel(1);
        let calls = Arc::new(AtomicUsize::new(0));
        let worker_stop = Arc::clone(&stop);
        let worker_calls = Arc::clone(&calls);
        let worker = thread::Builder::new()
            .name("mod-studio-monitor-test".to_owned())
            .spawn(move || {
                run_mod_studio_monitor(
                    worker_stop.as_ref(),
                    receiver,
                    Duration::from_millis(1),
                    || {
                        worker_calls.fetch_add(1, Ordering::AcqRel);
                    },
                );
            })
            .expect("spawn monitor test worker");
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        while calls.load(Ordering::Acquire) < 2 {
            assert!(std::time::Instant::now() < deadline, "monitor did not poll");
            thread::yield_now();
        }

        stop.store(true, Ordering::Release);
        wake.try_send(()).expect("wake monitor shutdown");
        worker.join().expect("join monitor");
        let stopped_at = calls.load(Ordering::Acquire);
        thread::sleep(Duration::from_millis(3));
        assert_eq!(calls.load(Ordering::Acquire), stopped_at);
    }

    #[test]
    fn application_owned_monitor_shutdown_is_idempotent() {
        let stop = Arc::new(AtomicBool::new(false));
        let (wake, receiver) = sync_channel(1);
        let worker_stop = Arc::clone(&stop);
        let worker = thread::Builder::new()
            .name("mod-studio-monitor-shutdown-test".to_owned())
            .spawn(move || {
                run_mod_studio_monitor(
                    worker_stop.as_ref(),
                    receiver,
                    Duration::from_secs(1),
                    || {},
                );
            })
            .expect("spawn monitor shutdown test worker");
        let runtime = ModStudioMonitorRuntime {
            stop,
            wake,
            worker: Mutex::new(Some(worker)),
        };

        assert!(runtime.shutdown());
        assert!(!runtime.shutdown());
    }

    #[test]
    fn stable_bootstrap_failure_is_invoked_once_for_the_same_context() {
        let now = Instant::now();
        let mut policy =
            BootstrapRetryPolicy::new(Duration::from_millis(10), Duration::from_millis(40));
        let calls = AtomicUsize::new(0);
        let operation = || {
            calls.fetch_add(1, Ordering::AcqRel);
            BootstrapAttempt {
                state: ModStudioPollState::BootstrapFailed(
                    ModsPluginBootstrapErrorCode::ModuleImageMismatch,
                ),
                disposition: BootstrapRetryDisposition::StableFailure,
            }
        };

        assert_eq!(
            policy.evaluate(7_u64, now, operation),
            ModStudioPollState::BootstrapFailed(ModsPluginBootstrapErrorCode::ModuleImageMismatch)
        );
        assert_eq!(
            policy.evaluate(
                7_u64,
                now.checked_add(Duration::from_secs(60))
                    .expect("fixture instant"),
                operation,
            ),
            ModStudioPollState::BootstrapFailed(ModsPluginBootstrapErrorCode::ModuleImageMismatch)
        );
        assert_eq!(calls.load(Ordering::Acquire), 1);
    }

    #[test]
    fn bootstrap_context_change_releases_the_stable_failure_latch() {
        let now = Instant::now();
        let mut policy =
            BootstrapRetryPolicy::new(Duration::from_millis(10), Duration::from_millis(40));
        let calls = AtomicUsize::new(0);
        let operation = || {
            calls.fetch_add(1, Ordering::AcqRel);
            BootstrapAttempt {
                state: ModStudioPollState::BootstrapFailed(
                    ModsPluginBootstrapErrorCode::InitializationFailed,
                ),
                disposition: BootstrapRetryDisposition::StableFailure,
            }
        };

        let proxy_context = ModStudioBootstrapContext {
            settings_generation: 7,
            loading_method: ModStudioLoadingMethod::Proxy,
            risk_acknowledged: true,
            target: ModStudioBootstrapTarget::NoGame,
        };
        let _ = policy.evaluate(proxy_context.clone(), now, operation);
        let _ = policy.evaluate(proxy_context.clone(), now, operation);

        let mut new_settings_generation = proxy_context.clone();
        new_settings_generation.settings_generation += 1;
        let _ = policy.evaluate(new_settings_generation, now, operation);

        let mut new_loading_method = proxy_context;
        new_loading_method.loading_method = ModStudioLoadingMethod::Loader;
        let _ = policy.evaluate(new_loading_method, now, operation);
        assert_eq!(calls.load(Ordering::Acquire), 3);
    }

    #[test]
    fn recoverable_bootstrap_attempts_use_bounded_exponential_backoff() {
        let now = Instant::now();
        let base = Duration::from_millis(10);
        let max = Duration::from_millis(40);
        let mut policy = BootstrapRetryPolicy::new(base, max);
        let calls = AtomicUsize::new(0);
        let operation = || {
            calls.fetch_add(1, Ordering::AcqRel);
            BootstrapAttempt {
                state: ModStudioPollState::Waiting,
                disposition: BootstrapRetryDisposition::Recoverable,
            }
        };

        let _ = policy.evaluate(7_u64, now, operation);
        let _ = policy.evaluate(
            7_u64,
            now.checked_add(base - Duration::from_millis(1))
                .expect("fixture instant"),
            operation,
        );
        assert_eq!(calls.load(Ordering::Acquire), 1);
        let second = now.checked_add(base).expect("fixture instant");
        let _ = policy.evaluate(7_u64, second, operation);
        assert_eq!(calls.load(Ordering::Acquire), 2);
        let _ = policy.evaluate(
            7_u64,
            second
                .checked_add(base.saturating_mul(2) - Duration::from_millis(1))
                .expect("fixture instant"),
            operation,
        );
        assert_eq!(calls.load(Ordering::Acquire), 2);
        let third = second
            .checked_add(base.saturating_mul(2))
            .expect("fixture instant");
        let _ = policy.evaluate(7_u64, third, operation);
        assert_eq!(calls.load(Ordering::Acquire), 3);
        assert_eq!(policy.next_attempt, third.checked_add(max));
    }

    #[test]
    fn poll_runtime_shutdown_waits_for_at_most_one_finite_operation() {
        let (entered_tx, entered_rx) = mpsc::channel();
        let (_release_tx, release_rx) = mpsc::channel::<()>();
        let release_rx = Arc::new(Mutex::new(release_rx));
        let calls = Arc::new(AtomicUsize::new(0));
        let runtime = ModStudioPollRuntime::start_with(
            {
                let release_rx = Arc::clone(&release_rx);
                let calls = Arc::clone(&calls);
                Arc::new(move |_| {
                    calls.fetch_add(1, Ordering::AcqRel);
                    entered_tx.send(()).expect("publish operation entry");
                    let _ = release_rx
                        .lock()
                        .expect("release receiver lock")
                        .recv_timeout(Duration::from_millis(50));
                    ModStudioPollState::Waiting
                })
            },
            &SystemThreadSpawner,
        )
        .expect("start finite poll lane");
        runtime
            .handle()
            .request(AppState::default())
            .expect("queue finite poll");
        entered_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("operation entered");

        let shutdown_started = Instant::now();
        drop(runtime);
        assert!(shutdown_started.elapsed() < Duration::from_secs(1));
        assert_eq!(calls.load(Ordering::Acquire), 1);
    }

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

    #[test]
    fn proxy_bootstrap_requires_explicit_success_and_preserves_typed_failures() {
        assert_eq!(
            classify_proxy_bootstrap(Ok(Some(ModsPluginInitializeOutcome::Started))),
            ModStudioPollState::LoaderPresent
        );
        assert_eq!(
            classify_proxy_bootstrap(Ok(Some(ModsPluginInitializeOutcome::AlreadyRunning))),
            ModStudioPollState::LoaderPresent
        );
        assert_eq!(
            classify_proxy_bootstrap(Ok(None)),
            ModStudioPollState::Waiting
        );
        assert_eq!(
            classify_proxy_bootstrap(Err(ModsPluginBootstrapError::NotGameHost)),
            ModStudioPollState::BootstrapFailed(ModsPluginBootstrapErrorCode::NotGameHost)
        );
        assert_eq!(
            classify_proxy_bootstrap(Err(ModsPluginBootstrapError::InitializationFailed)),
            ModStudioPollState::BootstrapFailed(ModsPluginBootstrapErrorCode::InitializationFailed)
        );

        let in_progress = classify_proxy_bootstrap_attempt(Err(
            ModsPluginBootstrapError::LocalBootstrapInProgress,
        ));
        assert_eq!(in_progress.state, ModStudioPollState::Waiting);
        assert_eq!(
            in_progress.disposition,
            BootstrapRetryDisposition::Recoverable
        );
        let stable =
            classify_proxy_bootstrap_attempt(Err(ModsPluginBootstrapError::InitializationFailed));
        assert_eq!(
            stable.state,
            ModStudioPollState::BootstrapFailed(ModsPluginBootstrapErrorCode::InitializationFailed)
        );
        assert_eq!(stable.disposition, BootstrapRetryDisposition::StableFailure);
    }

    #[test]
    fn native_start_requires_persisted_risk_acknowledgement() {
        assert_eq!(
            mod_studio_start_precondition(ModStudioLoadingMethod::Proxy, false),
            Some(ModStudioPollState::AcknowledgementRequired)
        );
        assert_eq!(
            mod_studio_start_precondition(ModStudioLoadingMethod::Loader, false),
            Some(ModStudioPollState::AcknowledgementRequired)
        );
        assert_eq!(
            mod_studio_start_precondition(ModStudioLoadingMethod::Loader, true),
            Some(ModStudioPollState::Waiting)
        );
        assert_eq!(
            mod_studio_start_precondition(ModStudioLoadingMethod::Proxy, true),
            None
        );
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
