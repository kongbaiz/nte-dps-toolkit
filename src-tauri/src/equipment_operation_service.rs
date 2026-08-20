use std::sync::{Mutex, MutexGuard};

use nte_dps_tool::{
    engine::model::HtItemNetId,
    platform::mods_plugin::{
        ModsPluginClient, ModsPluginOperation, ModsPluginReceiveError, ModsPluginResponse,
        ModsPluginSubmitError,
    },
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EmptyCurtainOperationState {
    pub status: &'static str,
    pub message_key: &'static str,
    pub message_arguments: Vec<String>,
    request_id: Option<u64>,
}

impl Default for EmptyCurtainOperationState {
    fn default() -> Self {
        Self {
            status: "idle",
            message_key: "No equipment operation is pending",
            message_arguments: Vec::new(),
            request_id: None,
        }
    }
}

impl EmptyCurtainOperationState {
    fn pending(request_id: u64) -> Self {
        Self {
            status: "pending",
            message_key: "Sending equipment request...",
            message_arguments: Vec::new(),
            request_id: Some(request_id),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EquipmentOperationSnapshot {
    pub operation: EmptyCurtainOperationState,
    pub revision: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EquipmentOperationError {
    Busy,
    Disconnected,
    Unavailable,
}

struct EquipmentOperationInner {
    operation: EmptyCurtainOperationState,
    revision: u64,
    client: ModsPluginClient,
    unavailable: bool,
}

impl Default for EquipmentOperationInner {
    fn default() -> Self {
        Self {
            operation: EmptyCurtainOperationState::default(),
            revision: 0,
            client: ModsPluginClient::new(),
            unavailable: false,
        }
    }
}

/// One recovery and transaction domain for equipment operation state.
///
/// `ModsPluginClient::submit` and `try_recv` are bounded channel operations;
/// the worker performs the actual plugin RPC outside this lock. Keeping the
/// client, request id, projection, and revision in one mutex makes publication
/// atomic and prevents a fast response from racing ahead of pending state.
#[derive(Default)]
pub(crate) struct EquipmentOperationService {
    state: Mutex<EquipmentOperationInner>,
}

impl EquipmentOperationService {
    pub(crate) fn submit(
        &self,
        character: HtItemNetId,
        operation: ModsPluginOperation,
    ) -> Result<u64, EquipmentOperationError> {
        self.submit_with(character, operation, ModsPluginClient::submit)
    }

    pub(crate) fn poll_snapshot(
        &self,
    ) -> Result<EquipmentOperationSnapshot, EquipmentOperationError> {
        self.poll_snapshot_with(ModsPluginClient::try_recv)
    }

    fn submit_with<F>(
        &self,
        character: HtItemNetId,
        operation: ModsPluginOperation,
        submit: F,
    ) -> Result<u64, EquipmentOperationError>
    where
        F: FnOnce(
            &mut ModsPluginClient,
            HtItemNetId,
            ModsPluginOperation,
        ) -> Result<u64, ModsPluginSubmitError>,
    {
        let mut state = self.lock_ready()?;
        if state.operation.request_id.is_some() {
            return Err(EquipmentOperationError::Busy);
        }
        let request_id =
            submit(&mut state.client, character, operation).map_err(|error| match error {
                ModsPluginSubmitError::Busy => EquipmentOperationError::Busy,
                ModsPluginSubmitError::Disconnected => EquipmentOperationError::Disconnected,
            })?;
        state.operation = EmptyCurtainOperationState::pending(request_id);
        state.revision = next_revision(state.revision);
        Ok(request_id)
    }

    fn poll_snapshot_with<F>(
        &self,
        receive: F,
    ) -> Result<EquipmentOperationSnapshot, EquipmentOperationError>
    where
        F: FnOnce(&ModsPluginClient) -> Result<Option<ModsPluginResponse>, ModsPluginReceiveError>,
    {
        let mut state = self.lock_ready()?;
        let received = receive(&state.client);
        let next = match received {
            Ok(Some(response)) if state.operation.request_id == Some(response.request_id) => {
                Some(operation_from_response(response))
            }
            Ok(Some(_) | None) => None,
            Err(ModsPluginReceiveError::Disconnected) if state.operation.request_id.is_some() => {
                Some(EmptyCurtainOperationState {
                    status: "error",
                    message_key: "Mod loader worker disconnected",
                    message_arguments: Vec::new(),
                    request_id: None,
                })
            }
            Err(ModsPluginReceiveError::Disconnected) => None,
        };
        if let Some(next) = next.filter(|next| *next != state.operation) {
            state.operation = next;
            state.revision = next_revision(state.revision);
        }
        Ok(EquipmentOperationSnapshot {
            operation: state.operation.clone(),
            revision: state.revision,
        })
    }

    fn lock_ready(
        &self,
    ) -> Result<MutexGuard<'_, EquipmentOperationInner>, EquipmentOperationError> {
        match self.state.lock() {
            Ok(state) if !state.unavailable => Ok(state),
            Ok(_) => Err(EquipmentOperationError::Unavailable),
            Err(poison) => {
                let mut state = poison.into_inner();
                state.unavailable = true;
                self.state.clear_poison();
                Err(EquipmentOperationError::Unavailable)
            }
        }
    }
}

fn operation_from_response(response: ModsPluginResponse) -> EmptyCurtainOperationState {
    match response.status {
        Ok(0) => EmptyCurtainOperationState {
            status: "success",
            message_key: "Equipment RPC dispatched; waiting for game synchronization",
            message_arguments: Vec::new(),
            request_id: None,
        },
        Ok(1) => EmptyCurtainOperationState {
            status: "success",
            message_key: "Equipment request passed plugin dry-run validation",
            message_arguments: Vec::new(),
            request_id: None,
        },
        Ok(status) => EmptyCurtainOperationState {
            status: "error",
            message_key: "Mod loader rejected the request (status {})",
            message_arguments: vec![status.to_string()],
            request_id: None,
        },
        Err(_) => EmptyCurtainOperationState {
            status: "error",
            message_key: "Mod loader is unavailable",
            message_arguments: Vec::new(),
            request_id: None,
        },
    }
}

const fn next_revision(revision: u64) -> u64 {
    revision.wrapping_add(1)
}

#[cfg(test)]
mod tests {
    use std::{
        panic::{AssertUnwindSafe, catch_unwind},
        sync::{Arc, Barrier, mpsc},
        thread,
        time::Duration,
    };

    use super::*;

    fn character() -> HtItemNetId {
        HtItemNetId { solt: 7, serial: 9 }
    }

    fn operation() -> ModsPluginOperation {
        ModsPluginOperation::UnequipAll
    }

    fn idle_snapshot(service: &EquipmentOperationService) -> EquipmentOperationSnapshot {
        service
            .poll_snapshot_with(|_| Ok(None))
            .expect("equipment state is available")
    }

    #[test]
    fn accepted_submit_bumps_once_and_pending_is_busy() {
        let service = EquipmentOperationService::default();
        assert_eq!(
            service.submit_with(character(), operation(), |_, _, _| Ok(41)),
            Ok(41)
        );
        let pending = idle_snapshot(&service);
        assert_eq!(pending.revision, 1);
        assert_eq!(pending.operation.request_id, Some(41));
        assert_eq!(
            service.submit_with(character(), operation(), |_, _, _| {
                panic!("pending operation must not call the submit hook")
            }),
            Err(EquipmentOperationError::Busy)
        );
        assert_eq!(idle_snapshot(&service).revision, 1);
    }

    #[test]
    fn concurrent_submits_admit_at_most_one_request() {
        let service = Arc::new(EquipmentOperationService::default());
        let start = Arc::new(Barrier::new(3));
        let mut workers = Vec::new();
        for request_id in [51, 52] {
            let service = Arc::clone(&service);
            let start = Arc::clone(&start);
            workers.push(thread::spawn(move || {
                start.wait();
                service.submit_with(character(), operation(), |_, _, _| Ok(request_id))
            }));
        }
        start.wait();
        let results = workers
            .into_iter()
            .map(|worker| worker.join().expect("submit worker exits"))
            .collect::<Vec<_>>();
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|result| **result == Err(EquipmentOperationError::Busy))
                .count(),
            1
        );
        assert_eq!(idle_snapshot(&service).revision, 1);
    }

    #[test]
    fn a_fast_response_cannot_be_polled_before_pending_is_published() {
        let service = Arc::new(EquipmentOperationService::default());
        let submit_entered = Arc::new(Barrier::new(2));
        let release_submit = Arc::new(Barrier::new(2));
        let submit_service = Arc::clone(&service);
        let worker_entered = Arc::clone(&submit_entered);
        let worker_release = Arc::clone(&release_submit);
        let submit = thread::spawn(move || {
            submit_service.submit_with(character(), operation(), |_, _, _| {
                worker_entered.wait();
                worker_release.wait();
                Ok(56)
            })
        });
        submit_entered.wait();

        let (poll_entered_tx, poll_entered_rx) = mpsc::channel();
        let poll_service = Arc::clone(&service);
        let poll = thread::spawn(move || {
            poll_service.poll_snapshot_with(|_| {
                poll_entered_tx
                    .send(())
                    .expect("poll observation receiver stays alive");
                Ok(Some(ModsPluginResponse {
                    request_id: 56,
                    status: Ok(0),
                }))
            })
        });
        assert!(
            poll_entered_rx
                .recv_timeout(Duration::from_millis(25))
                .is_err(),
            "poll hook must wait for the atomic pending commit"
        );
        release_submit.wait();
        assert_eq!(submit.join().expect("submit worker exits"), Ok(56));
        poll_entered_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("poll resumes after pending commit");
        let completed = poll
            .join()
            .expect("poll worker exits")
            .expect("equipment state is available");
        assert_eq!(completed.revision, 2);
        assert_eq!(completed.operation.status, "success");
    }

    #[test]
    fn failed_submission_and_empty_poll_do_not_bump() {
        let service = EquipmentOperationService::default();
        assert_eq!(
            service.submit_with(character(), operation(), |_, _, _| {
                Err(ModsPluginSubmitError::Disconnected)
            }),
            Err(EquipmentOperationError::Disconnected)
        );
        assert_eq!(idle_snapshot(&service).revision, 0);
    }

    #[test]
    fn stale_response_does_not_bump_or_clear_pending() {
        let service = EquipmentOperationService::default();
        service
            .submit_with(character(), operation(), |_, _, _| Ok(61))
            .expect("submission succeeds");
        let stale = service
            .poll_snapshot_with(|_| {
                Ok(Some(ModsPluginResponse {
                    request_id: 62,
                    status: Ok(0),
                }))
            })
            .expect("equipment state is available");
        assert_eq!(stale.revision, 1);
        assert_eq!(stale.operation.request_id, Some(61));
    }

    #[test]
    fn matching_response_bumps_once_and_redacts_plugin_detail() {
        let service = EquipmentOperationService::default();
        service
            .submit_with(character(), operation(), |_, _, _| Ok(71))
            .expect("submission succeeds");
        let completed = service
            .poll_snapshot_with(|_| {
                Ok(Some(ModsPluginResponse {
                    request_id: 71,
                    status: Err(r"C:\Users\private\plugin.pipe: token=secret".to_owned()),
                }))
            })
            .expect("equipment state is available");
        assert_eq!(completed.revision, 2);
        assert_eq!(completed.operation.message_key, "Mod loader is unavailable");
        assert!(completed.operation.message_arguments.is_empty());
        assert_eq!(completed.operation.request_id, None);
    }

    #[test]
    fn disconnected_poll_only_bumps_a_pending_operation() {
        let service = EquipmentOperationService::default();
        let idle = service
            .poll_snapshot_with(|_| Err(ModsPluginReceiveError::Disconnected))
            .expect("equipment state is available");
        assert_eq!(idle.revision, 0);

        service
            .submit_with(character(), operation(), |_, _, _| Ok(81))
            .expect("submission succeeds");
        let failed = service
            .poll_snapshot_with(|_| Err(ModsPluginReceiveError::Disconnected))
            .expect("equipment state is available");
        assert_eq!(failed.revision, 2);
        assert_eq!(failed.operation.status, "error");
        assert!(failed.operation.message_arguments.is_empty());
    }

    #[test]
    fn poisoned_state_is_sticky_and_never_calls_client_hooks() {
        let service = EquipmentOperationService::default();
        let poisoned = catch_unwind(AssertUnwindSafe(|| {
            let mut state = service.state.lock().expect("equipment state locks");
            state.operation = EmptyCurtainOperationState::pending(91);
            panic!("poison equipment state after a partial update");
        }));
        assert!(poisoned.is_err());

        assert_eq!(
            service.poll_snapshot_with(|_| {
                panic!("unavailable service must not call receive hook")
            }),
            Err(EquipmentOperationError::Unavailable)
        );
        assert_eq!(
            service.submit_with(character(), operation(), |_, _, _| {
                panic!("unavailable service must not call submit hook")
            }),
            Err(EquipmentOperationError::Unavailable)
        );
    }
}
