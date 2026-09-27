//! Capacity-one control relay on the existing native capture connection. The
//! capture owner drains notifications while a finite inventory request waits.
use crate::platform::{
    capture_pipe::CapturePipe,
    toolkit::{ToolkitClient, ToolkitError},
};
use serde_json::{Value, json};
use std::sync::{
    Arc, Mutex, OnceLock,
    atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc::{self, Receiver, SyncSender},
};
use std::time::{Duration, Instant};

#[derive(Default)]
pub struct Router {
    route: Mutex<Option<(u64, String, SyncSender<Pending>)>>,
    next: AtomicU64,
}
pub(crate) struct Pending {
    pub id: String,
    pub method: String,
    pub params: Value,
    pub deadline: Instant,
    pub canceled: Arc<AtomicBool>,
    pub reply: SyncSender<Result<Value, ToolkitError>>,
}
pub(crate) struct Registration {
    router: Arc<Router>,
    token: u64,
    pub receiver: Receiver<Pending>,
}
impl Drop for Registration {
    fn drop(&mut self) {
        if let Ok(mut route) = self.router.route.lock()
            && route.as_ref().is_some_and(|r| r.0 == self.token)
        {
            *route = None;
        }
        while let Ok(p) = self.receiver.try_recv() {
            let _ = p.reply.try_send(Err(ToolkitError::Cancelled));
        }
    }
}
impl Router {
    pub(crate) fn register(
        self: &Arc<Self>,
        identity: String,
    ) -> Result<Registration, ToolkitError> {
        let mut route = self.route.lock().map_err(|_| ToolkitError::Failed)?;
        if route.is_some() {
            return Err(ToolkitError::Busy);
        }
        let token = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let (tx, rx) = mpsc::sync_channel(1);
        *route = Some((token, identity, tx));
        Ok(Registration {
            router: self.clone(),
            token,
            receiver: rx,
        })
    }
    pub fn connect(&self, host: &ToolkitClient, pid: u32) -> Result<Client, ToolkitError> {
        let route = self.route.lock().map_err(|_| ToolkitError::Failed)?.clone();
        let transport = if let Some((_, owner, tx)) = route {
            if owner != host.identity() {
                return Err(ToolkitError::SessionChanged);
            }
            Transport::Relay(tx)
        } else {
            Transport::Direct(CapturePipe::open(host)?)
        };
        let mut client = Client {
            transport,
            stop: Arc::new(AtomicBool::new(false)),
        };
        if matches!(client.transport, Transport::Direct(_)) {
            let hello = client.call("hello", json!({"protocolVersion":1}))?;
            if hello["protocolVersion"] != 1 || hello["gamePid"] != pid || hello["ready"] != true {
                return Err(ToolkitError::Unavailable);
            }
            for cap in ["equipment.execute.v1", "inventory.snapshot.v1"] {
                if !hello["capabilities"]
                    .as_array()
                    .is_some_and(|a| a.iter().any(|x| x == cap))
                {
                    return Err(ToolkitError::Unsupported);
                }
            }
        }
        Ok(client)
    }
}
enum Transport {
    Direct(CapturePipe),
    Relay(SyncSender<Pending>),
}
pub struct Client {
    transport: Transport,
    stop: Arc<AtomicBool>,
}
static REQUEST_ID: AtomicU64 = AtomicU64::new(1);
static REQUEST_EPOCH: OnceLock<u128> = OnceLock::new();
pub fn request_id() -> String {
    format!(
        "equipment-{}-{}-{}",
        std::process::id(),
        REQUEST_EPOCH.get_or_init(|| std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()),
        REQUEST_ID.fetch_add(1, Ordering::Relaxed)
    )
}
pub(crate) fn response(v: Value, id: &str, method: &str) -> Result<Value, ToolkitError> {
    if v["jsonrpc"] != "2.0" || v["id"] != id {
        return Err(ToolkitError::InvalidProtocol);
    }
    if v.get("error").is_some() && v.get("result").is_some() {
        return Err(ToolkitError::InvalidProtocol);
    }
    if let Some(error) = v.get("error") {
        // A changing inventory can publish an unreadable intermediate snapshot;
        // this is not a lost connection or a rejected equipment operation.
        if method.starts_with("snapshot.")
            && matches!(
                error["message"].as_str(),
                Some("not_ready" | "snapshot_not_found")
            )
        {
            return Err(ToolkitError::SessionChanged);
        }
        return Err(match error["message"].as_str() {
            Some("control_busy" | "user_busy") => ToolkitError::Busy,
            Some("source_changed" | "stale") => ToolkitError::SessionChanged,
            Some("not_ready" | "user_plugin_unavailable") => ToolkitError::Unavailable,
            Some("control_timeout") => ToolkitError::Timeout,
            _ => ToolkitError::Failed,
        });
    }
    v.get("result")
        .cloned()
        .ok_or(ToolkitError::InvalidProtocol)
}
impl Client {
    pub fn with_cancel(mut self, stop: Arc<AtomicBool>) -> Self {
        self.stop = stop;
        self
    }
    pub fn call(&mut self, method: &str, params: Value) -> Result<Value, ToolkitError> {
        let id = request_id();
        self.call_with_id(&id, method, params)
    }
    pub fn call_with_id(
        &mut self,
        id: &str,
        method: &str,
        params: Value,
    ) -> Result<Value, ToolkitError> {
        if self.stop.load(Ordering::Acquire) {
            return Err(ToolkitError::Cancelled);
        }
        let deadline = Instant::now()
            + Duration::from_secs(if method == "snapshot.refresh" {
                130
            } else {
                10
            });
        match &self.transport {
            Transport::Direct(pipe) => {
                pipe.send(id, method, params)?;
                loop {
                    if self.stop.load(Ordering::Acquire) {
                        return Err(ToolkitError::Cancelled);
                    }
                    if let Some(bytes) = pipe.read(&self.stop)? {
                        let v: Value = serde_json::from_slice(&bytes)
                            .map_err(|_| ToolkitError::InvalidProtocol)?;
                        // Snapshot invalidations are notifications, never command replies.
                        if v.get("id").is_none() && v["method"] == "event.snapshot.changed" {
                            continue;
                        }
                        return response(v, id, method);
                    }
                    if Instant::now() >= deadline {
                        return Err(ToolkitError::Timeout);
                    }
                    std::thread::sleep(Duration::from_millis(2));
                }
            }
            Transport::Relay(tx) => {
                let (reply, rx) = mpsc::sync_channel(1);
                let canceled = Arc::new(AtomicBool::new(false));
                tx.try_send(Pending {
                    id: id.into(),
                    method: method.into(),
                    params,
                    deadline,
                    canceled: canceled.clone(),
                    reply,
                })
                .map_err(|e| match e {
                    mpsc::TrySendError::Full(_) => ToolkitError::Busy,
                    mpsc::TrySendError::Disconnected(_) => ToolkitError::Unavailable,
                })?;
                let result = loop {
                    if self.stop.load(Ordering::Acquire) {
                        break Err(ToolkitError::Cancelled);
                    }
                    if Instant::now() >= deadline {
                        break Err(ToolkitError::Timeout);
                    }
                    match rx.recv_timeout(Duration::from_millis(25)) {
                        Ok(v) => break Ok(v),
                        Err(mpsc::RecvTimeoutError::Timeout) => continue,
                        Err(mpsc::RecvTimeoutError::Disconnected) => {
                            break Err(ToolkitError::Unavailable);
                        }
                    }
                };
                canceled.store(true, Ordering::Release);
                result?
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn relay_is_bounded_and_drops_pending_work_on_owner_end() {
        let router = Arc::new(Router::default());
        let owner = router.register("p:1".into()).unwrap();
        assert!(matches!(
            router.register("p:1".into()),
            Err(ToolkitError::Busy)
        ));
        let tx = router.route.lock().unwrap().as_ref().unwrap().2.clone();
        let (reply, rx) = mpsc::sync_channel(1);
        let pending = || Pending {
            id: request_id(),
            method: "equipment.status".into(),
            params: json!({}),
            deadline: Instant::now() + Duration::from_secs(1),
            canceled: Arc::new(AtomicBool::new(false)),
            reply: reply.clone(),
        };
        tx.try_send(pending()).unwrap();
        assert!(matches!(
            tx.try_send(pending()),
            Err(mpsc::TrySendError::Full(_))
        ));
        drop(owner);
        assert_eq!(rx.recv().unwrap(), Err(ToolkitError::Cancelled));
        assert!(router.route.lock().unwrap().is_none());
        assert!(router.register("p:2".into()).is_ok());
    }
    #[test]
    fn wrong_receipts_and_native_errors_are_not_success() {
        assert_eq!(
            response(
                json!({"jsonrpc":"2.0","id":"foreign","result":{}}),
                "ours",
                "equipment.status"
            ),
            Err(ToolkitError::InvalidProtocol)
        );
        assert_eq!(
            response(
                json!({"jsonrpc":"2.0","id":"ours","error":{"message":"source_changed"}}),
                "ours",
                "equipment.status"
            ),
            Err(ToolkitError::SessionChanged)
        );
        assert_eq!(
            response(
                json!({"jsonrpc":"2.0","id":"ours","error":{},"result":{}}),
                "ours",
                "equipment.status"
            ),
            Err(ToolkitError::InvalidProtocol)
        );
    }
    #[test]
    fn snapshot_not_ready_is_retryable_but_equipment_not_ready_is_not() {
        let error = json!({"jsonrpc":"2.0","id":"r","error":{"code":-32001,"message":"not_ready"}});
        assert_eq!(
            response(error.clone(), "r", "snapshot.refresh"),
            Err(ToolkitError::SessionChanged)
        );
        assert_eq!(
            response(error.clone(), "r", "snapshot.page"),
            Err(ToolkitError::SessionChanged)
        );
        assert_eq!(
            response(error, "r", "equipment.status"),
            Err(ToolkitError::Unavailable)
        );
    }
    #[test]
    fn subscription_cancellation_cancels_queued_control_without_waiting_for_reply() {
        let (tx, rx) = mpsc::sync_channel(1);
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = stop.clone();
        let worker = std::thread::spawn(move || {
            let mut client = Client {
                transport: Transport::Relay(tx),
                stop: thread_stop,
            };
            client.call(
                "equipment.inspect",
                json!({"equipment":{"solt":1,"serial":2}}),
            )
        });
        let pending = rx.recv_timeout(Duration::from_secs(2)).unwrap();
        stop.store(true, Ordering::Release);
        assert_eq!(worker.join().unwrap(), Err(ToolkitError::Cancelled));
        assert!(pending.canceled.load(Ordering::Acquire));
    }
}
