use std::{
    io::{self, Write},
    marker::PhantomData,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        mpsc::{Receiver, RecvTimeoutError, SyncSender, TryRecvError, TrySendError, sync_channel},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use serde::Serialize;
use tauri::ipc::Channel;

use crate::{
    contract::{
        CommandError,
        stream::{
            MAX_EVENTS_PER_STREAM_DELIVERY, MAX_STREAM_DELIVERY_BYTES, StreamDeliveryBody,
            StreamKind, StreamReadySignal,
        },
    },
    state::{AppState, StreamRegistration, StreamRegistryError},
};

const STREAM_WORKER_THREADS: usize = 4;
const STREAM_REGISTRATION_CAPACITY: usize = 128;
const STREAM_WORK_QUEUE_CAPACITY: usize = 64;
const STREAM_COMPLETION_CAPACITY: usize = 64;
const TIMER_CANCELLATION_SCAN: Duration = Duration::from_millis(25);
const WORK_QUEUE_RETRY_DELAY: Duration = Duration::from_millis(2);
const REGISTRATION_ACK_TIMEOUT: Duration = Duration::from_secs(1);

static STREAM_RUNTIME: OnceLock<Mutex<Option<StreamRuntime>>> = OnceLock::new();
static NEXT_TASK_ID: AtomicU64 = AtomicU64::new(1);

pub(crate) fn is_valid_subscription_id(value: &str) -> bool {
    (1..=64).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

pub(crate) fn validate_subscription_id(value: &str) -> Result<(), CommandError> {
    if is_valid_subscription_id(value) {
        Ok(())
    } else {
        Err(CommandError::invalid_subscription_id())
    }
}

pub(crate) fn stream_registry_error(_: StreamRegistryError) -> CommandError {
    CommandError::stream_runtime_unavailable()
}

/// Idempotently stops and joins the shared timer/workers. The runtime slot is
/// detached before any join so a later test/app restart can initialize it again.
pub(crate) fn shutdown_polling_stream_runtime() -> bool {
    let Some(slot) = STREAM_RUNTIME.get() else {
        return false;
    };
    let runtime = match slot.lock() {
        Ok(mut slot) => slot.take(),
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

/// Result of one revision-aware projection pass.
///
/// `Events` preserves the projection's ordering. The runtime rechecks the owner
/// cancellation token after projection and immediately before every emission.
pub(crate) enum PollingStreamOutput<T> {
    NoChange,
    Event(T),
    Events(Vec<T>),
    Stop,
}

pub(crate) struct StreamDeliveryEndpoint {
    stream_kind: StreamKind,
    subscription_id: String,
    on_ready: Channel<StreamReadySignal>,
}

impl StreamDeliveryEndpoint {
    pub(crate) fn new(
        stream_kind: StreamKind,
        subscription_id: String,
        on_ready: Channel<StreamReadySignal>,
    ) -> Self {
        Self {
            stream_kind,
            subscription_id,
            on_ready,
        }
    }
}

/// Registers a revision-aware Channel poller on the shared stream runtime.
///
/// Lifecycle and backpressure contract:
/// - capacity: 128 active streams, 128 pending registrations, 64 queued polls;
/// - ordering: one in-flight poll per stream and ordered emission within a poll;
/// - work queue full: the due poll remains pending and is retried (coalesced tick);
/// - registration full/disconnect: registration fails closed and runs cleanup;
/// - output disconnect: the stream terminates and runs token-checked cleanup;
/// - producer threads: one timer plus four fixed workers for all subscriptions.
///
/// The timer owns only deadlines and task tokens. Projection and Channel sends
/// always execute on a worker, outside the timer thread and AppState stream lock.
pub(crate) fn spawn_polling_stream<T, Poll>(
    stream_name: &'static str,
    delivery: StreamDeliveryEndpoint,
    state: AppState,
    registration: StreamRegistration,
    interval_ms: u32,
    poll: Poll,
) -> Result<(), CommandError>
where
    T: Serialize + Send + 'static,
    Poll: FnMut(&AppState) -> PollingStreamOutput<T> + Send + 'static,
{
    let stop = registration.stop_token();
    let active = registration.activation_token();
    let cleanup_state = state.clone();
    let cleanup_registration = registration.clone();
    let worker_state = state.clone();
    let mut task = Some(ScheduledStream {
        task_id: NEXT_TASK_ID.fetch_add(1, Ordering::Relaxed),
        stream_name,
        owner_window: registration.owner_window().to_owned(),
        stream_key: registration.stream_key().to_owned(),
        generation: registration.generation(),
        stop,
        active,
        next_tick: Instant::now(),
        interval: Duration::from_millis(u64::from(interval_ms.max(1))),
        work: Some(Box::new(PollingWork {
            state: worker_state,
            registration: registration.clone(),
            stream_kind: delivery.stream_kind,
            subscription_id: delivery.subscription_id,
            on_ready: delivery.on_ready,
            poll,
            output: PhantomData,
        })),
        cleanup: Some(Box::new(move || {
            if cleanup_state.finish_stream(&cleanup_registration).is_err() {
                log::warn!("Stream registry cleanup reset after an interrupted update");
            }
        })),
    });

    let slot = STREAM_RUNTIME.get_or_init(|| Mutex::new(None));
    let runtime_handle = match slot.lock() {
        Ok(mut runtime_slot) => {
            let initialized = if runtime_slot.is_none() {
                match StreamRuntime::start_with(&SystemThreadSpawner) {
                    Ok(started) => {
                        *runtime_slot = Some(started);
                        true
                    }
                    Err(_) => false,
                }
            } else {
                true
            };
            if initialized {
                runtime_slot
                    .as_ref()
                    .map(StreamRuntime::registration_handle)
            } else {
                None
            }
        }
        Err(mut poison) => {
            let stale = poison.get_mut().take();
            slot.clear_poison();
            drop(poison);
            drop(stale);
            None
        }
    };
    let registered = match (runtime_handle, task.take()) {
        (Some(runtime), Some(task)) if runtime.is_healthy() => runtime.register(task).is_ok(),
        _ => false,
    };
    drop(task);
    if !registered {
        registration.cancel();
        if state.finish_stream(&registration).is_err() {
            log::warn!("Stream registry rollback reset after an interrupted update");
        }
        return Err(CommandError::stream_runtime_unavailable());
    }

    match state.activate_stream(&registration) {
        Ok(true) => {}
        Ok(false) | Err(_) => {
            registration.cancel();
            if state.finish_stream(&registration).is_err() {
                log::warn!("Stream registry activation reset after an interrupted update");
            }
            return Err(CommandError::stream_runtime_unavailable());
        }
    }
    Ok(())
}

trait StreamWork: Send {
    fn execute(&mut self, stop: &AtomicBool) -> bool;
}

struct PollingWork<T, Poll> {
    state: AppState,
    registration: StreamRegistration,
    stream_kind: StreamKind,
    subscription_id: String,
    on_ready: Channel<StreamReadySignal>,
    poll: Poll,
    output: PhantomData<fn() -> T>,
}

impl<T, Poll> StreamWork for PollingWork<T, Poll>
where
    T: Serialize + Send + 'static,
    Poll: FnMut(&AppState) -> PollingStreamOutput<T> + Send + 'static,
{
    fn execute(&mut self, stop: &AtomicBool) -> bool {
        if stop.load(Ordering::Acquire) {
            return false;
        }
        match self.state.stream_delivery_outstanding(&self.registration) {
            Ok(true) => return true,
            Ok(false) => {}
            Err(_) => return false,
        }
        let reservation = match self
            .state
            .reserve_stream_delivery_capacity(&self.registration)
        {
            Ok(Some(reservation)) => reservation,
            Ok(None) => return true,
            Err(_) => return false,
        };
        let output = (self.poll)(&self.state);
        if stop.load(Ordering::Acquire) {
            return false;
        }

        let events = match output {
            PollingStreamOutput::NoChange => return true,
            PollingStreamOutput::Stop => return false,
            PollingStreamOutput::Event(event) => vec![event],
            PollingStreamOutput::Events(events) if events.is_empty() => return true,
            PollingStreamOutput::Events(events)
                if events.len() <= MAX_EVENTS_PER_STREAM_DELIVERY =>
            {
                events
            }
            PollingStreamOutput::Events(_) => return false,
        };
        let bytes = match serialize_stream_events(events) {
            Ok(bytes) => bytes,
            Err(_) => return false,
        };
        let sequence =
            match self
                .state
                .stage_stream_delivery(&self.registration, bytes, reservation)
            {
                Ok(Some(sequence)) => sequence,
                Ok(None) => return !stop.load(Ordering::Acquire),
                Err(_) => return false,
            };
        if stop.load(Ordering::Acquire) {
            self.registration.cancel();
            return false;
        }
        let signal = StreamReadySignal::new(
            self.stream_kind,
            self.subscription_id.clone(),
            self.registration.generation(),
            sequence,
        );
        if self.on_ready.send(signal).is_err() {
            self.registration.cancel();
            return false;
        }
        !stop.load(Ordering::Acquire)
    }
}

pub(crate) fn serialize_stream_events<T: Serialize>(events: Vec<T>) -> io::Result<Vec<u8>> {
    let mut output = BoundedJsonBuffer::new(MAX_STREAM_DELIVERY_BYTES);
    serde_json::to_writer(&mut output, &StreamDeliveryBody::new(events))
        .map_err(io::Error::other)?;
    Ok(output.into_inner())
}

struct BoundedJsonBuffer {
    bytes: Vec<u8>,
    limit: usize,
}

impl BoundedJsonBuffer {
    fn new(limit: usize) -> Self {
        Self {
            bytes: Vec::with_capacity(limit.min(4 * 1024)),
            limit,
        }
    }

    fn into_inner(self) -> Vec<u8> {
        self.bytes
    }
}

impl Write for BoundedJsonBuffer {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let Some(next_len) = self.bytes.len().checked_add(buffer.len()) else {
            return Err(io::Error::other("stream delivery exceeds byte budget"));
        };
        if next_len > self.limit {
            return Err(io::Error::other("stream delivery exceeds byte budget"));
        }
        self.bytes.extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct StreamRuntime {
    registration_sender: SyncSender<TimerCommand>,
    healthy: Arc<AtomicBool>,
    shutdown: Arc<AtomicBool>,
    timer_handle: Option<JoinHandle<()>>,
    worker_handles: Vec<JoinHandle<()>>,
    #[cfg(test)]
    work_queue_full_count: Arc<AtomicUsize>,
}

#[derive(Clone)]
struct StreamRuntimeRegistrationHandle {
    registration_sender: SyncSender<TimerCommand>,
    healthy: Arc<AtomicBool>,
}

impl StreamRuntimeRegistrationHandle {
    fn register(&self, task: ScheduledStream) -> Result<(), ()> {
        if !self.is_healthy() {
            drop(task);
            return Err(());
        }
        let (acknowledgement, acknowledged) = sync_channel(1);
        match self.registration_sender.try_send(TimerCommand::Register {
            task,
            acknowledgement,
        }) {
            Ok(()) => match acknowledged.recv_timeout(REGISTRATION_ACK_TIMEOUT) {
                Ok(true) => Ok(()),
                Ok(false) | Err(_) => Err(()),
            },
            Err(TrySendError::Full(message) | TrySendError::Disconnected(message)) => {
                drop(message);
                Err(())
            }
        }
    }

    fn is_healthy(&self) -> bool {
        self.healthy.load(Ordering::Acquire)
    }
}

impl StreamRuntime {
    fn start_with(spawner: &dyn ThreadSpawner) -> io::Result<Self> {
        let (registration_sender, registration_receiver) =
            sync_channel(STREAM_REGISTRATION_CAPACITY);
        let (work_sender, work_receiver) = sync_channel(STREAM_WORK_QUEUE_CAPACITY);
        let (completion_sender, completion_receiver) = sync_channel(STREAM_COMPLETION_CAPACITY);
        let work_receiver = Arc::new(Mutex::new(work_receiver));
        let shutdown = Arc::new(AtomicBool::new(false));
        let healthy = Arc::new(AtomicBool::new(true));
        let work_queue_full_count = Arc::new(AtomicUsize::new(0));
        let mut worker_handles = Vec::with_capacity(STREAM_WORKER_THREADS);

        for index in 0..STREAM_WORKER_THREADS {
            let worker_receiver = Arc::clone(&work_receiver);
            let worker_completion = completion_sender.clone();
            let worker_shutdown = Arc::clone(&shutdown);
            let worker_healthy = Arc::clone(&healthy);
            let job_shutdown = Arc::clone(&worker_shutdown);
            let job_healthy = Arc::clone(&worker_healthy);
            let job = Box::new(move || {
                let result = catch_unwind(AssertUnwindSafe(|| {
                    run_worker(worker_receiver, worker_completion, worker_shutdown)
                }));
                if result.is_err() {
                    log::error!("stream worker terminated after an unexpected panic");
                }
                if !job_shutdown.load(Ordering::Acquire) {
                    job_healthy.store(false, Ordering::Release);
                }
            });
            match spawner.spawn(format!("nte-stream-worker-{index}"), job) {
                Ok(handle) => worker_handles.push(handle),
                Err(error) => {
                    shutdown.store(true, Ordering::Release);
                    drop(work_sender);
                    drop(completion_sender);
                    for handle in worker_handles {
                        let _ = handle.join();
                    }
                    return Err(error);
                }
            }
        }
        drop(completion_sender);

        let timer_shutdown = Arc::clone(&shutdown);
        let timer_healthy = Arc::clone(&healthy);
        let timer_full_count = Arc::clone(&work_queue_full_count);
        let job_shutdown = Arc::clone(&timer_shutdown);
        let job_healthy = Arc::clone(&timer_healthy);
        let timer_job = Box::new(move || {
            let result = catch_unwind(AssertUnwindSafe(|| {
                run_timer(
                    registration_receiver,
                    completion_receiver,
                    work_sender,
                    timer_shutdown,
                    timer_full_count,
                )
            }));
            if result.is_err() {
                log::error!("stream timer terminated after an unexpected panic");
            }
            if !job_shutdown.load(Ordering::Acquire) {
                job_healthy.store(false, Ordering::Release);
            }
        });
        let timer_handle = match spawner.spawn("nte-stream-timer".to_owned(), timer_job) {
            Ok(handle) => handle,
            Err(error) => {
                shutdown.store(true, Ordering::Release);
                drop(registration_sender);
                for handle in worker_handles {
                    let _ = handle.join();
                }
                return Err(error);
            }
        };

        Ok(Self {
            registration_sender,
            healthy,
            shutdown,
            timer_handle: Some(timer_handle),
            worker_handles,
            #[cfg(test)]
            work_queue_full_count,
        })
    }

    #[cfg(test)]
    fn register(&self, task: ScheduledStream) -> Result<(), ()> {
        self.registration_handle().register(task)
    }

    #[cfg(test)]
    fn is_healthy(&self) -> bool {
        self.healthy.load(Ordering::Acquire)
    }

    fn registration_handle(&self) -> StreamRuntimeRegistrationHandle {
        StreamRuntimeRegistrationHandle {
            registration_sender: self.registration_sender.clone(),
            healthy: Arc::clone(&self.healthy),
        }
    }

    #[cfg(test)]
    fn work_queue_full_count(&self) -> usize {
        self.work_queue_full_count.load(Ordering::Acquire)
    }

    #[cfg(test)]
    fn shutdown(self) {
        drop(self);
    }
}

impl Drop for StreamRuntime {
    fn drop(&mut self) {
        self.healthy.store(false, Ordering::Release);
        self.shutdown.store(true, Ordering::Release);
        let _ = self.registration_sender.try_send(TimerCommand::Shutdown);
        if let Some(handle) = self.timer_handle.take() {
            let _ = handle.join();
        }
        for handle in self.worker_handles.drain(..) {
            let _ = handle.join();
        }
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

enum TimerCommand {
    Register {
        task: ScheduledStream,
        acknowledgement: SyncSender<bool>,
    },
    Shutdown,
}

struct ScheduledStream {
    task_id: u64,
    stream_name: &'static str,
    owner_window: String,
    stream_key: String,
    generation: u64,
    stop: Arc<AtomicBool>,
    active: Arc<AtomicBool>,
    next_tick: Instant,
    interval: Duration,
    work: Option<Box<dyn StreamWork>>,
    cleanup: Option<Box<dyn FnOnce() + Send>>,
}

impl Drop for ScheduledStream {
    fn drop(&mut self) {
        if let Some(cleanup) = self.cleanup.take() {
            cleanup();
        }
    }
}

struct WorkItem {
    task_id: u64,
    stream_name: &'static str,
    stop: Arc<AtomicBool>,
    work: Box<dyn StreamWork>,
}

struct WorkCompletion {
    task_id: u64,
    work: Box<dyn StreamWork>,
    keep_running: bool,
}

fn run_worker(
    work_receiver: Arc<Mutex<Receiver<WorkItem>>>,
    completion_sender: SyncSender<WorkCompletion>,
    shutdown: Arc<AtomicBool>,
) {
    while !shutdown.load(Ordering::Acquire) {
        let received = {
            let Ok(receiver) = work_receiver.lock() else {
                return;
            };
            receiver.recv_timeout(TIMER_CANCELLATION_SCAN)
        };
        let mut item = match received {
            Ok(item) => item,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => return,
        };

        let keep_running =
            match catch_unwind(AssertUnwindSafe(|| item.work.execute(item.stop.as_ref()))) {
                Ok(keep_running) => keep_running,
                Err(_) => {
                    log::error!("stream poll panicked: {}", item.stream_name);
                    false
                }
            };
        if completion_sender
            .send(WorkCompletion {
                task_id: item.task_id,
                work: item.work,
                keep_running,
            })
            .is_err()
        {
            return;
        }
    }
}

fn run_timer(
    registration_receiver: Receiver<TimerCommand>,
    completion_receiver: Receiver<WorkCompletion>,
    work_sender: SyncSender<WorkItem>,
    shutdown: Arc<AtomicBool>,
    work_queue_full_count: Arc<AtomicUsize>,
) {
    let mut tasks = Vec::<ScheduledStream>::new();
    let mut dispatch_cursor = 0_usize;

    loop {
        if shutdown.load(Ordering::Acquire) {
            break;
        }

        match drain_completions(&mut tasks, &completion_receiver) {
            QueueState::Open => {}
            QueueState::Disconnected => break,
        }
        match drain_registrations(&mut tasks, &registration_receiver) {
            TimerState::Running => {}
            TimerState::Shutdown => break,
            TimerState::Disconnected => break,
        }
        remove_cancelled_idle_tasks(&mut tasks);

        if !dispatch_due_tasks(
            &mut tasks,
            &work_sender,
            &mut dispatch_cursor,
            &work_queue_full_count,
        ) {
            break;
        }

        let wait = next_timer_wait(&tasks);
        match registration_receiver.recv_timeout(wait) {
            Ok(TimerCommand::Register {
                task,
                acknowledgement,
            }) => handle_registration(&mut tasks, task, acknowledgement),
            Ok(TimerCommand::Shutdown) => break,
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }

    for task in &tasks {
        task.stop.store(true, Ordering::Release);
    }
}

enum QueueState {
    Open,
    Disconnected,
}

enum TimerState {
    Running,
    Shutdown,
    Disconnected,
}

fn drain_completions(
    tasks: &mut Vec<ScheduledStream>,
    receiver: &Receiver<WorkCompletion>,
) -> QueueState {
    loop {
        match receiver.try_recv() {
            Ok(completion) => complete_task(tasks, completion),
            Err(TryRecvError::Empty) => return QueueState::Open,
            Err(TryRecvError::Disconnected) => return QueueState::Disconnected,
        }
    }
}

fn drain_registrations(
    tasks: &mut Vec<ScheduledStream>,
    receiver: &Receiver<TimerCommand>,
) -> TimerState {
    loop {
        match receiver.try_recv() {
            Ok(TimerCommand::Register {
                task,
                acknowledgement,
            }) => handle_registration(tasks, task, acknowledgement),
            Ok(TimerCommand::Shutdown) => return TimerState::Shutdown,
            Err(TryRecvError::Empty) => return TimerState::Running,
            Err(TryRecvError::Disconnected) => return TimerState::Disconnected,
        }
    }
}

fn handle_registration(
    tasks: &mut Vec<ScheduledStream>,
    task: ScheduledStream,
    acknowledgement: SyncSender<bool>,
) {
    let task_id = task.task_id;
    let accepted = register_task(tasks, task);
    if acknowledgement.send(accepted).is_err() && accepted {
        if let Some(index) = tasks.iter().position(|task| task.task_id == task_id) {
            tasks[index].stop.store(true, Ordering::Release);
            if tasks[index].work.is_some() {
                tasks.swap_remove(index);
            }
        }
    }
}

fn register_task(tasks: &mut Vec<ScheduledStream>, task: ScheduledStream) -> bool {
    if task.stop.load(Ordering::Acquire)
        || tasks.iter().any(|existing| {
            same_stream_identity(existing, &task) && existing.generation >= task.generation
        })
    {
        return false;
    }
    tasks.push(task);
    cancel_superseded_active_tasks(tasks);
    remove_cancelled_idle_tasks(tasks);
    true
}

fn same_stream_identity(left: &ScheduledStream, right: &ScheduledStream) -> bool {
    left.owner_window == right.owner_window && left.stream_key == right.stream_key
}

fn complete_task(tasks: &mut Vec<ScheduledStream>, completion: WorkCompletion) {
    let Some(index) = tasks
        .iter()
        .position(|task| task.task_id == completion.task_id)
    else {
        return;
    };
    let task = &mut tasks[index];
    task.work = Some(completion.work);
    if !completion.keep_running || task.stop.load(Ordering::Acquire) {
        tasks.swap_remove(index);
    } else {
        task.next_tick = Instant::now() + task.interval;
    }
}

fn remove_cancelled_idle_tasks(tasks: &mut Vec<ScheduledStream>) {
    let mut index = 0;
    while index < tasks.len() {
        if tasks[index].stop.load(Ordering::Acquire) && tasks[index].work.is_some() {
            tasks.swap_remove(index);
        } else {
            index += 1;
        }
    }
}

fn cancel_superseded_active_tasks(tasks: &[ScheduledStream]) {
    for candidate in tasks
        .iter()
        .filter(|task| task.active.load(Ordering::Acquire) && !task.stop.load(Ordering::Acquire))
    {
        for existing in tasks.iter().filter(|existing| {
            same_stream_identity(existing, candidate) && existing.generation < candidate.generation
        }) {
            existing.stop.store(true, Ordering::Release);
        }
    }
}

fn dispatch_due_tasks(
    tasks: &mut [ScheduledStream],
    sender: &SyncSender<WorkItem>,
    cursor: &mut usize,
    work_queue_full_count: &AtomicUsize,
) -> bool {
    if tasks.is_empty() {
        *cursor = 0;
        return true;
    }

    let now = Instant::now();
    cancel_superseded_active_tasks(tasks);
    let task_count = tasks.len();
    let start = *cursor % task_count;
    for offset in 0..task_count {
        let index = (start + offset) % task_count;
        if tasks[index].stop.load(Ordering::Acquire)
            || tasks[index].work.is_none()
            || tasks[index].next_tick > now
        {
            continue;
        }
        if !tasks[index].active.load(Ordering::Acquire) {
            tasks[index].next_tick = now + WORK_QUEUE_RETRY_DELAY;
            continue;
        }
        let replacement_waits_for_in_flight = tasks.iter().enumerate().any(|(other, task)| {
            other != index && same_stream_identity(task, &tasks[index]) && task.work.is_none()
        });
        if replacement_waits_for_in_flight {
            tasks[index].next_tick = now + WORK_QUEUE_RETRY_DELAY;
            continue;
        }

        let Some(work) = tasks[index].work.take() else {
            continue;
        };
        let item = WorkItem {
            task_id: tasks[index].task_id,
            stream_name: tasks[index].stream_name,
            stop: Arc::clone(&tasks[index].stop),
            work,
        };
        match sender.try_send(item) {
            Ok(()) => {
                *cursor = (index + 1) % task_count;
            }
            Err(TrySendError::Full(item)) => {
                tasks[index].work = Some(item.work);
                tasks[index].next_tick = now + WORK_QUEUE_RETRY_DELAY;
                work_queue_full_count.fetch_add(1, Ordering::Relaxed);
                *cursor = (index + 1) % task_count;
                break;
            }
            Err(TrySendError::Disconnected(item)) => {
                tasks[index].work = Some(item.work);
                return false;
            }
        }
    }
    true
}

fn next_timer_wait(tasks: &[ScheduledStream]) -> Duration {
    let now = Instant::now();
    tasks
        .iter()
        .filter(|task| task.work.is_some())
        .map(|task| task.next_tick.saturating_duration_since(now))
        .min()
        .unwrap_or(TIMER_CANCELLATION_SCAN)
        .min(TIMER_CANCELLATION_SCAN)
        .max(Duration::from_millis(1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Barrier, mpsc};

    static TEST_LOCK: Mutex<()> = Mutex::new(());

    struct TestWork(Box<dyn FnMut(&AtomicBool) -> bool + Send>);

    impl StreamWork for TestWork {
        fn execute(&mut self, stop: &AtomicBool) -> bool {
            (self.0)(stop)
        }
    }

    fn test_task(
        stream_name: &'static str,
        stream_key: impl Into<String>,
        stop: Arc<AtomicBool>,
        work: impl FnMut(&AtomicBool) -> bool + Send + 'static,
        cleanups: Arc<AtomicUsize>,
    ) -> ScheduledStream {
        let task_id = NEXT_TASK_ID.fetch_add(1, Ordering::Relaxed);
        ScheduledStream {
            task_id,
            stream_name,
            owner_window: "test-window".to_owned(),
            stream_key: stream_key.into(),
            generation: task_id,
            stop,
            active: Arc::new(AtomicBool::new(true)),
            next_tick: Instant::now(),
            interval: Duration::from_millis(5),
            work: Some(Box::new(TestWork(Box::new(work)))),
            cleanup: Some(Box::new(move || {
                cleanups.fetch_add(1, Ordering::AcqRel);
            })),
        }
    }

    fn wait_until(timeout: Duration, condition: impl Fn() -> bool) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if condition() {
                return true;
            }
            thread::sleep(Duration::from_millis(2));
        }
        condition()
    }

    #[test]
    fn subscription_ids_share_one_bounded_ascii_policy() {
        assert!(validate_subscription_id("stream_01").is_ok());
        assert!(validate_subscription_id("").is_err());
        assert!(validate_subscription_id("stream/01").is_err());
        assert!(validate_subscription_id(&"a".repeat(65)).is_err());
    }

    #[test]
    fn dropping_a_queued_task_runs_cleanup() {
        let _guard = TEST_LOCK
            .lock()
            .expect("stream runtime test lock must remain healthy");
        let cleanups = Arc::new(AtomicUsize::new(0));
        let task = test_task(
            "drop-test",
            "drop-test",
            Arc::new(AtomicBool::new(false)),
            |_| true,
            Arc::clone(&cleanups),
        );
        drop(task);
        assert_eq!(cleanups.load(Ordering::Acquire), 1);
    }

    #[test]
    fn blocking_projection_does_not_block_timer_or_a_sibling_stream() {
        let _guard = TEST_LOCK
            .lock()
            .expect("stream runtime test lock must remain healthy");
        let runtime = StreamRuntime::start_with(&SystemThreadSpawner).unwrap();
        let cleanups = Arc::new(AtomicUsize::new(0));
        let (entered_sender, entered_receiver) = mpsc::channel();
        let (release_sender, release_receiver) = mpsc::channel();
        let (sibling_sender, sibling_receiver) = mpsc::channel();

        runtime
            .register(test_task(
                "blocked",
                "blocked",
                Arc::new(AtomicBool::new(false)),
                move |_| {
                    let _ = entered_sender.send(());
                    let _ = release_receiver.recv();
                    false
                },
                Arc::clone(&cleanups),
            ))
            .unwrap();
        runtime
            .register(test_task(
                "sibling",
                "sibling",
                Arc::new(AtomicBool::new(false)),
                move |_| {
                    let _ = sibling_sender.send(());
                    false
                },
                Arc::clone(&cleanups),
            ))
            .unwrap();

        entered_receiver
            .recv_timeout(Duration::from_secs(1))
            .unwrap();
        assert!(
            sibling_receiver
                .recv_timeout(Duration::from_secs(1))
                .is_ok(),
            "a blocked projection stalled its sibling"
        );
        release_sender.send(()).unwrap();
        runtime.shutdown();
        assert_eq!(cleanups.load(Ordering::Acquire), 2);
    }

    #[test]
    fn replacement_cancellation_after_projection_prevents_late_send() {
        let _guard = TEST_LOCK
            .lock()
            .expect("stream runtime test lock must remain healthy");
        let runtime = StreamRuntime::start_with(&SystemThreadSpawner).unwrap();
        let cleanups = Arc::new(AtomicUsize::new(0));
        let old_stop = Arc::new(AtomicBool::new(false));
        let new_stop = Arc::new(AtomicBool::new(false));
        let old_sends = Arc::new(AtomicUsize::new(0));
        let new_sends = Arc::new(AtomicUsize::new(0));
        let (entered_sender, entered_receiver) = mpsc::channel();
        let (release_sender, release_receiver) = mpsc::channel();

        let old_send_count = Arc::clone(&old_sends);
        let old_work = TestWork(Box::new(move |stop: &AtomicBool| {
            let _ = entered_sender.send(());
            let _ = release_receiver.recv();
            if stop.load(Ordering::Acquire) {
                return false;
            }
            old_send_count.fetch_add(1, Ordering::AcqRel);
            false
        }));
        let old_task_id = NEXT_TASK_ID.fetch_add(1, Ordering::Relaxed);
        runtime
            .register(ScheduledStream {
                task_id: old_task_id,
                stream_name: "old",
                owner_window: "test-window".to_owned(),
                stream_key: "replacement".to_owned(),
                generation: old_task_id,
                stop: Arc::clone(&old_stop),
                active: Arc::new(AtomicBool::new(true)),
                next_tick: Instant::now(),
                interval: Duration::from_secs(1),
                work: Some(Box::new(old_work)),
                cleanup: Some(Box::new({
                    let cleanups = Arc::clone(&cleanups);
                    move || {
                        cleanups.fetch_add(1, Ordering::AcqRel);
                    }
                })),
            })
            .unwrap();
        entered_receiver
            .recv_timeout(Duration::from_secs(1))
            .unwrap();

        let new_send_count = Arc::clone(&new_sends);
        runtime
            .register(test_task(
                "new",
                "replacement",
                Arc::clone(&new_stop),
                move |_| {
                    new_send_count.fetch_add(1, Ordering::AcqRel);
                    false
                },
                Arc::clone(&cleanups),
            ))
            .unwrap();
        assert!(wait_until(Duration::from_secs(1), || old_stop.load(Ordering::Acquire)));
        release_sender.send(()).unwrap();
        assert!(wait_until(Duration::from_secs(1), || new_sends
            .load(Ordering::Acquire)
            == 1));

        runtime.shutdown();
        assert_eq!(old_sends.load(Ordering::Acquire), 0);
        assert_eq!(new_sends.load(Ordering::Acquire), 1);
        assert_eq!(cleanups.load(Ordering::Acquire), 2);
    }

    #[test]
    fn stale_late_registration_cannot_cancel_a_newer_generation() {
        let _guard = TEST_LOCK
            .lock()
            .expect("stream runtime test lock must remain healthy");
        let runtime = StreamRuntime::start_with(&SystemThreadSpawner).unwrap();
        let cleanups = Arc::new(AtomicUsize::new(0));
        let newer_stop = Arc::new(AtomicBool::new(false));
        let newer_active = Arc::new(AtomicBool::new(false));
        let newer_ran = Arc::new(AtomicBool::new(false));
        let mut newer = test_task(
            "newer",
            "generation-race",
            Arc::clone(&newer_stop),
            {
                let newer_ran = Arc::clone(&newer_ran);
                move |_| {
                    newer_ran.store(true, Ordering::Release);
                    false
                }
            },
            Arc::clone(&cleanups),
        );
        newer.generation = 2;
        newer.active = Arc::clone(&newer_active);
        runtime.register(newer).unwrap();

        let stale_stop = Arc::new(AtomicBool::new(false));
        let mut stale = test_task(
            "stale",
            "generation-race",
            Arc::clone(&stale_stop),
            |_| false,
            Arc::clone(&cleanups),
        );
        stale.generation = 1;
        assert!(runtime.register(stale).is_err());
        assert!(!newer_stop.load(Ordering::Acquire));
        newer_active.store(true, Ordering::Release);
        assert!(wait_until(Duration::from_secs(1), || newer_ran.load(Ordering::Acquire)));

        runtime.shutdown();
        assert_eq!(cleanups.load(Ordering::Acquire), 2);
    }

    #[test]
    fn pending_replacement_does_not_cancel_current_before_activation() {
        let _guard = TEST_LOCK
            .lock()
            .expect("stream runtime test lock must remain healthy");
        let runtime = StreamRuntime::start_with(&SystemThreadSpawner).unwrap();
        let cleanups = Arc::new(AtomicUsize::new(0));
        let current_stop = Arc::new(AtomicBool::new(false));
        let mut current = test_task(
            "current",
            "transactional-replacement",
            Arc::clone(&current_stop),
            |_| true,
            Arc::clone(&cleanups),
        );
        current.generation = 1;
        current.next_tick = Instant::now() + Duration::from_secs(1);
        runtime.register(current).unwrap();

        let replacement_stop = Arc::new(AtomicBool::new(false));
        let mut replacement = test_task(
            "replacement",
            "transactional-replacement",
            Arc::clone(&replacement_stop),
            |_| true,
            Arc::clone(&cleanups),
        );
        replacement.generation = 2;
        replacement.active = Arc::new(AtomicBool::new(false));
        runtime.register(replacement).unwrap();

        assert!(!current_stop.load(Ordering::Acquire));
        replacement_stop.store(true, Ordering::Release);
        assert!(wait_until(Duration::from_secs(1), || cleanups
            .load(Ordering::Acquire)
            >= 1));
        assert!(!current_stop.load(Ordering::Acquire));

        runtime.shutdown();
        assert_eq!(cleanups.load(Ordering::Acquire), 2);
    }

    #[test]
    fn panicking_poll_stops_only_that_task_and_worker_survives() {
        let _guard = TEST_LOCK
            .lock()
            .expect("stream runtime test lock must remain healthy");
        let runtime = StreamRuntime::start_with(&SystemThreadSpawner).unwrap();
        let cleanups = Arc::new(AtomicUsize::new(0));
        let sibling_ran = Arc::new(AtomicBool::new(false));

        runtime
            .register(test_task(
                "panicking",
                "panicking",
                Arc::new(AtomicBool::new(false)),
                |_| panic!("test poll panic"),
                Arc::clone(&cleanups),
            ))
            .unwrap();
        let sibling_flag = Arc::clone(&sibling_ran);
        runtime
            .register(test_task(
                "sibling",
                "panic-sibling",
                Arc::new(AtomicBool::new(false)),
                move |_| {
                    sibling_flag.store(true, Ordering::Release);
                    false
                },
                Arc::clone(&cleanups),
            ))
            .unwrap();

        assert!(wait_until(Duration::from_secs(1), || sibling_ran
            .load(Ordering::Acquire)));
        assert!(runtime.is_healthy());
        runtime.shutdown();
        assert_eq!(cleanups.load(Ordering::Acquire), 2);
    }

    #[test]
    fn full_work_queue_retries_without_losing_task() {
        let _guard = TEST_LOCK
            .lock()
            .expect("stream runtime test lock must remain healthy");
        let runtime = StreamRuntime::start_with(&SystemThreadSpawner).unwrap();
        let cleanups = Arc::new(AtomicUsize::new(0));
        let barrier = Arc::new(Barrier::new(STREAM_WORKER_THREADS + 1));
        let release = Arc::new(AtomicBool::new(false));

        for index in 0..STREAM_WORKER_THREADS {
            let worker_barrier = Arc::clone(&barrier);
            let worker_release = Arc::clone(&release);
            runtime
                .register(test_task(
                    "queue-blocker",
                    format!("queue-blocker-{index}"),
                    Arc::new(AtomicBool::new(false)),
                    move |_| {
                        worker_barrier.wait();
                        while !worker_release.load(Ordering::Acquire) {
                            thread::sleep(Duration::from_millis(1));
                        }
                        false
                    },
                    Arc::clone(&cleanups),
                ))
                .unwrap();
        }
        barrier.wait();

        for index in 0..STREAM_WORK_QUEUE_CAPACITY {
            runtime
                .register(test_task(
                    "queued",
                    format!("queued-{index}"),
                    Arc::new(AtomicBool::new(false)),
                    |_| false,
                    Arc::clone(&cleanups),
                ))
                .unwrap();
        }
        let target_ran = Arc::new(AtomicBool::new(false));
        let target_flag = Arc::clone(&target_ran);
        runtime
            .register(test_task(
                "full-retry-target",
                "full-retry-target",
                Arc::new(AtomicBool::new(false)),
                move |_| {
                    target_flag.store(true, Ordering::Release);
                    false
                },
                Arc::clone(&cleanups),
            ))
            .unwrap();

        assert!(wait_until(Duration::from_secs(1), || runtime
            .work_queue_full_count()
            > 0));
        release.store(true, Ordering::Release);
        assert!(wait_until(Duration::from_secs(2), || target_ran
            .load(Ordering::Acquire)));
        runtime.shutdown();

        assert_eq!(
            cleanups.load(Ordering::Acquire),
            STREAM_WORKER_THREADS + STREAM_WORK_QUEUE_CAPACITY + 1
        );
    }

    struct FailingSpawner {
        calls: AtomicUsize,
        fail_at: usize,
        active: Arc<AtomicUsize>,
    }

    impl ThreadSpawner for FailingSpawner {
        fn spawn(
            &self,
            name: String,
            job: Box<dyn FnOnce() + Send + 'static>,
        ) -> io::Result<JoinHandle<()>> {
            let call = self.calls.fetch_add(1, Ordering::AcqRel);
            if call == self.fail_at {
                return Err(io::Error::other("injected thread spawn failure"));
            }
            let active = Arc::clone(&self.active);
            thread::Builder::new().name(name).spawn(move || {
                active.fetch_add(1, Ordering::AcqRel);
                job();
                active.fetch_sub(1, Ordering::AcqRel);
            })
        }
    }

    #[test]
    fn partial_initialization_cleans_threads_and_later_start_retries() {
        let _guard = TEST_LOCK
            .lock()
            .expect("stream runtime test lock must remain healthy");
        let active = Arc::new(AtomicUsize::new(0));
        let spawner = FailingSpawner {
            calls: AtomicUsize::new(0),
            fail_at: 2,
            active: Arc::clone(&active),
        };

        assert!(StreamRuntime::start_with(&spawner).is_err());
        assert_eq!(active.load(Ordering::Acquire), 0);

        let runtime = StreamRuntime::start_with(&SystemThreadSpawner).unwrap();
        assert!(runtime.is_healthy());
        runtime.shutdown();
    }

    #[test]
    fn poisoned_runtime_slot_fails_one_registration_then_allows_retry() {
        let _guard = TEST_LOCK
            .lock()
            .expect("stream runtime test lock must remain healthy");
        shutdown_polling_stream_runtime();
        let slot = STREAM_RUNTIME.get_or_init(|| Mutex::new(None));
        let _ = thread::Builder::new()
            .name("nte-stream-slot-poison-test".to_owned())
            .spawn(move || {
                let _slot = slot.lock().expect("healthy runtime slot");
                panic!("poison stream runtime slot");
            })
            .expect("spawn poison fixture")
            .join();

        let state = AppState::default();
        let kind = StreamKind::Technical;
        let key = kind.stream_key("slot-retry");
        let first = state
            .reserve_stream("hud", &key)
            .expect("reserve failed registration");
        assert!(
            spawn_polling_stream(
                "slot-poison",
                StreamDeliveryEndpoint::new(
                    kind,
                    "slot-retry".to_owned(),
                    Channel::new(|_| Ok(())),
                ),
                state.clone(),
                first,
                1,
                |_| PollingStreamOutput::<()>::NoChange,
            )
            .is_err()
        );
        assert_eq!(state.stream_registry_len(), Ok(0));

        let retry = state
            .reserve_stream("hud", &key)
            .expect("reserve retry registration");
        spawn_polling_stream(
            "slot-retry",
            StreamDeliveryEndpoint::new(kind, "slot-retry".to_owned(), Channel::new(|_| Ok(()))),
            state.clone(),
            retry,
            1,
            |_| PollingStreamOutput::<()>::NoChange,
        )
        .expect("retry after poison cleanup");
        state.stop_stream("hud", &key).expect("stop retry stream");
        assert!(shutdown_polling_stream_runtime());
    }

    #[test]
    fn shared_runtime_shutdown_is_idempotent_and_drains_registered_tasks() {
        let _guard = TEST_LOCK
            .lock()
            .expect("stream runtime test lock must remain healthy");
        shutdown_polling_stream_runtime();
        let cleanups = Arc::new(AtomicUsize::new(0));
        let runtime = StreamRuntime::start_with(&SystemThreadSpawner).unwrap();
        let mut task = test_task(
            "shutdown",
            "shutdown",
            Arc::new(AtomicBool::new(false)),
            |_| true,
            Arc::clone(&cleanups),
        );
        task.active = Arc::new(AtomicBool::new(false));
        runtime.register(task).unwrap();
        let slot = STREAM_RUNTIME.get_or_init(|| Mutex::new(None));
        *slot
            .lock()
            .expect("shared runtime slot must remain healthy") = Some(runtime);

        assert!(shutdown_polling_stream_runtime());
        assert!(!shutdown_polling_stream_runtime());
        assert_eq!(cleanups.load(Ordering::Acquire), 1);
    }

    #[test]
    fn output_disconnect_finishes_the_exact_registry_generation() {
        let _guard = TEST_LOCK
            .lock()
            .expect("stream runtime test lock must remain healthy");
        shutdown_polling_stream_runtime();
        let state = AppState::default();
        let stream_kind = StreamKind::Technical;
        let registration = state
            .reserve_stream("hud", &stream_kind.stream_key("disconnect"))
            .expect("reserve disconnect stream");
        let disconnected = Channel::new(|_| Err(io::Error::other("disconnected").into()));

        spawn_polling_stream(
            "disconnect",
            StreamDeliveryEndpoint::new(stream_kind, "disconnect".to_owned(), disconnected),
            state.clone(),
            registration,
            1,
            |_| PollingStreamOutput::Event(()),
        )
        .expect("register disconnect stream");

        assert!(wait_until(Duration::from_secs(1), || {
            state.stream_registry_len() == Ok(0)
        }));
        assert!(shutdown_polling_stream_runtime());
    }

    #[test]
    fn one_delivery_remains_in_flight_until_the_exact_ack() {
        let _guard = TEST_LOCK
            .lock()
            .expect("stream runtime test lock must remain healthy");
        shutdown_polling_stream_runtime();
        let state = AppState::default();
        let stream_kind = StreamKind::Technical;
        let subscription_id = "ack-gate";
        let stream_key = stream_kind.stream_key(subscription_id);
        let registration = state
            .reserve_stream("hud", &stream_key)
            .expect("reserve acknowledged stream");
        let identity = registration.clone();
        let polls = Arc::new(AtomicUsize::new(0));
        let poll_count = Arc::clone(&polls);
        let ready = Arc::new(AtomicUsize::new(0));
        let ready_count = Arc::clone(&ready);
        let on_ready = Channel::new(move |_| {
            ready_count.fetch_add(1, Ordering::AcqRel);
            Ok(())
        });

        spawn_polling_stream(
            "ack-gate",
            StreamDeliveryEndpoint::new(stream_kind, subscription_id.to_owned(), on_ready),
            state.clone(),
            registration,
            1,
            move |_| {
                let current = poll_count.fetch_add(1, Ordering::AcqRel) + 1;
                PollingStreamOutput::Event(current)
            },
        )
        .expect("register acknowledged stream");

        assert!(wait_until(Duration::from_secs(1), || ready
            .load(Ordering::Acquire)
            == 1));
        thread::sleep(Duration::from_millis(20));
        assert_eq!(polls.load(Ordering::Acquire), 1);
        let bytes = state
            .take_stream_delivery("hud", &stream_key, identity.generation(), 1)
            .expect("read first delivery");
        assert!(
            String::from_utf8(bytes)
                .expect("JSON bytes")
                .contains("\"events\":[1]")
        );
        assert!(
            state
                .ack_stream_delivery("hud", &stream_key, identity.generation(), 1)
                .expect("ack first delivery")
        );
        assert!(wait_until(Duration::from_secs(1), || ready
            .load(Ordering::Acquire)
            == 2));
        assert_eq!(polls.load(Ordering::Acquire), 2);

        state
            .stop_stream("hud", &stream_key)
            .expect("stop acknowledged stream");
        assert!(wait_until(Duration::from_secs(1), || state
            .stream_registry_len()
            == Ok(0)));
        assert!(shutdown_polling_stream_runtime());
    }

    #[test]
    fn oversized_delivery_is_rejected_before_it_can_be_staged() {
        let oversized = "x".repeat(MAX_STREAM_DELIVERY_BYTES);
        assert!(serialize_stream_events(vec![oversized]).is_err());
    }
}
