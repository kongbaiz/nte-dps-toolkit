//! Frontend-neutral ownership for one live capture and its authoritative
//! [`CombatState`].
//!
//! The service keeps Npcap setup and parser work off UI threads, routes every
//! [`EngineEvent`] through the shared reducer, and exposes read-only state
//! access plus stable lifecycle categories. Frontends remain responsible for
//! translating those categories at their display boundary.

use std::{
    collections::HashMap,
    path::Path,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicU64, Ordering},
    },
    thread,
};

use crossbeam_channel::{Receiver, Sender, unbounded};

use super::{
    CoreError, CoreErrorCode,
    capture::{CaptureController, CaptureControllerOptions},
    reducer::{CoreSignal, apply_engine_event},
};
use crate::{
    engine::{
        model::{CharacterInfo, CombatState, EngineEvent},
        parser::{AbilityCatalog, CHARACTER_DATA_PATH, load_characters},
    },
    storage::{ability_names, i18n::Language},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiveCapturePhase {
    Idle,
    Starting,
    Running,
    Stopping,
    Stopped,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiveCaptureIssue {
    Start(CoreErrorCode),
    RuntimeWarning,
    RuntimeError,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LiveCaptureStatus {
    pub phase: LiveCapturePhase,
    pub issue: Option<LiveCaptureIssue>,
}

impl Default for LiveCaptureStatus {
    fn default() -> Self {
        Self {
            phase: LiveCapturePhase::Idle,
            issue: None,
        }
    }
}

#[derive(Clone, Default)]
pub struct LiveCaptureResources {
    pub characters: Arc<HashMap<u32, CharacterInfo>>,
    pub ability_catalog: Arc<AbilityCatalog>,
}

impl LiveCaptureResources {
    pub fn load(language: Language) -> (Self, Vec<String>) {
        let mut warnings = Vec::new();
        let characters = match load_characters(Path::new(CHARACTER_DATA_PATH)) {
            Ok(characters) => Arc::new(characters),
            Err(error) => {
                warnings.push(error.to_string());
                Arc::new(HashMap::new())
            }
        };
        let (ability_catalog, ability_warning) = ability_names::init(language);
        warnings.extend(ability_warning);
        (
            Self {
                characters,
                ability_catalog,
            },
            warnings,
        )
    }
}

#[derive(Clone)]
pub struct LiveCaptureService(Arc<LiveCaptureInner>);

struct LiveCaptureInner {
    state: Mutex<CombatState>,
    controller: Mutex<CaptureController>,
    status: Mutex<LiveCaptureStatus>,
    revision: AtomicU64,
    sender: Sender<EngineEvent>,
    receiver: Mutex<Option<Receiver<EngineEvent>>>,
    resources: LiveCaptureResources,
}

impl LiveCaptureService {
    pub fn new(resources: LiveCaptureResources) -> Self {
        let (sender, receiver) = unbounded();
        Self(Arc::new(LiveCaptureInner {
            state: Mutex::new(CombatState::default()),
            controller: Mutex::new(CaptureController::default()),
            status: Mutex::new(LiveCaptureStatus::default()),
            revision: AtomicU64::new(0),
            sender,
            receiver: Mutex::new(Some(receiver)),
            resources,
        }))
    }

    pub fn status(&self) -> LiveCaptureStatus {
        *self
            .0
            .status
            .lock()
            .expect("live capture status lock poisoned")
    }

    /// Returns a cheap monotonic marker for capture state that can affect
    /// frontend projections.
    pub fn revision(&self) -> u64 {
        self.0.revision.load(Ordering::Acquire)
    }

    pub fn with_state<T>(&self, read: impl FnOnce(&CombatState) -> T) -> T {
        let state = self
            .0
            .state
            .lock()
            .expect("live capture state lock poisoned");
        read(&state)
    }

    pub fn request_start(&self, options: CaptureControllerOptions) -> Result<(), CoreError> {
        self.ensure_event_worker()?;
        {
            let mut status = self
                .0
                .status
                .lock()
                .expect("live capture status lock poisoned");
            match status.phase {
                LiveCapturePhase::Idle | LiveCapturePhase::Stopped | LiveCapturePhase::Failed => {
                    *status = LiveCaptureStatus {
                        phase: LiveCapturePhase::Starting,
                        issue: None,
                    };
                }
                LiveCapturePhase::Starting
                | LiveCapturePhase::Running
                | LiveCapturePhase::Stopping => {
                    return Err(CoreError::new(
                        CoreErrorCode::CaptureAlreadyRunning,
                        "capture start is already active",
                    ));
                }
            }
        }
        self.0.bump_revision();

        let service = self.clone();
        thread::Builder::new()
            .name("nte-live-capture-start".to_owned())
            .spawn(move || service.finish_start(options))
            .map(|_| ())
            .map_err(|error| {
                self.record_start_failure(CoreErrorCode::SystemProbeFailed);
                CoreError::new(CoreErrorCode::SystemProbeFailed, error.to_string())
            })
    }

    pub fn request_stop(&self) -> Result<(), CoreError> {
        let should_spawn = {
            let mut status = self
                .0
                .status
                .lock()
                .expect("live capture status lock poisoned");
            match status.phase {
                LiveCapturePhase::Starting => {
                    status.phase = LiveCapturePhase::Stopping;
                    false
                }
                LiveCapturePhase::Running | LiveCapturePhase::Failed => {
                    status.phase = LiveCapturePhase::Stopping;
                    true
                }
                LiveCapturePhase::Stopping => return Ok(()),
                LiveCapturePhase::Idle | LiveCapturePhase::Stopped => {
                    return Err(CoreError::new(
                        CoreErrorCode::CaptureNotRunning,
                        "capture is not running",
                    ));
                }
            }
        };
        self.0.bump_revision();

        if should_spawn {
            self.spawn_stop_worker()?;
        }
        Ok(())
    }

    fn ensure_event_worker(&self) -> Result<(), CoreError> {
        let Some(receiver) = self
            .0
            .receiver
            .lock()
            .expect("live capture receiver lock poisoned")
            .take()
        else {
            return Ok(());
        };
        let weak = Arc::downgrade(&self.0);
        let worker_receiver = receiver.clone();
        match thread::Builder::new()
            .name("nte-live-engine-events".to_owned())
            .spawn(move || engine_event_loop(weak, worker_receiver))
        {
            Ok(_) => Ok(()),
            Err(error) => {
                *self
                    .0
                    .receiver
                    .lock()
                    .expect("live capture receiver lock poisoned") = Some(receiver);
                Err(CoreError::new(
                    CoreErrorCode::SystemProbeFailed,
                    error.to_string(),
                ))
            }
        }
    }

    fn finish_start(&self, options: CaptureControllerOptions) {
        let result = {
            // Holding the state lock keeps the event worker from applying the
            // first packet until a successful start has atomically reset the
            // previous combat session.
            let mut state = self
                .0
                .state
                .lock()
                .expect("live capture state lock poisoned");
            let result = self
                .0
                .controller
                .lock()
                .expect("live capture controller lock poisoned")
                .start(
                    options,
                    Arc::clone(&self.0.resources.characters),
                    Arc::clone(&self.0.resources.ability_catalog),
                    self.0.sender.clone(),
                );
            if result.is_ok() {
                *state = CombatState::default();
            }
            result
        };

        match result {
            Ok(()) => {
                let should_stop = {
                    let mut status = self
                        .0
                        .status
                        .lock()
                        .expect("live capture status lock poisoned");
                    if status.phase == LiveCapturePhase::Stopping {
                        true
                    } else {
                        *status = LiveCaptureStatus {
                            phase: LiveCapturePhase::Running,
                            issue: None,
                        };
                        false
                    }
                };
                self.0.bump_revision();
                if should_stop && self.spawn_stop_worker().is_err() {
                    self.record_start_failure(CoreErrorCode::SystemProbeFailed);
                }
            }
            Err(error) => self.record_start_failure(error.code),
        }
    }

    fn spawn_stop_worker(&self) -> Result<(), CoreError> {
        let service = self.clone();
        thread::Builder::new()
            .name("nte-live-capture-stop".to_owned())
            .spawn(move || service.finish_stop())
            .map(|_| ())
            .map_err(|error| {
                self.record_start_failure(CoreErrorCode::SystemProbeFailed);
                CoreError::new(CoreErrorCode::SystemProbeFailed, error.to_string())
            })
    }

    fn finish_stop(&self) {
        let mut controller = self
            .0
            .controller
            .lock()
            .expect("live capture controller lock poisoned");
        if controller.is_running() {
            controller
                .stop()
                .expect("running live capture controller must stop");
        }
        drop(controller);

        let mut status = self
            .0
            .status
            .lock()
            .expect("live capture status lock poisoned");
        if status.phase != LiveCapturePhase::Failed {
            *status = LiveCaptureStatus {
                phase: LiveCapturePhase::Stopped,
                issue: None,
            };
        }
        drop(status);
        self.0.bump_revision();
    }

    fn record_start_failure(&self, code: CoreErrorCode) {
        *self
            .0
            .status
            .lock()
            .expect("live capture status lock poisoned") = LiveCaptureStatus {
            phase: LiveCapturePhase::Failed,
            issue: Some(LiveCaptureIssue::Start(code)),
        };
        self.0.bump_revision();
    }
}

impl LiveCaptureInner {
    fn bump_revision(&self) {
        self.revision.fetch_add(1, Ordering::AcqRel);
    }

    fn process_event(&self, event: EngineEvent) {
        let signal = {
            let mut state = self.state.lock().expect("live capture state lock poisoned");
            apply_engine_event(&mut state, event)
        };

        let affects_frontend_projection = match signal {
            CoreSignal::StateChanged => true,
            CoreSignal::InventoryReplaced
            | CoreSignal::InventoryCharactersReplaced
            | CoreSignal::DebugPacket
            | CoreSignal::PacketObserved
            | CoreSignal::ModScript(_) => false,
            CoreSignal::Status(_) => {
                let mut status = self
                    .status
                    .lock()
                    .expect("live capture status lock poisoned");
                if status.phase == LiveCapturePhase::Starting {
                    status.phase = LiveCapturePhase::Running;
                }
                true
            }
            CoreSignal::Warning(_) => {
                self.status
                    .lock()
                    .expect("live capture status lock poisoned")
                    .issue = Some(LiveCaptureIssue::RuntimeWarning);
                true
            }
            CoreSignal::Error(_) => {
                *self
                    .status
                    .lock()
                    .expect("live capture status lock poisoned") = LiveCaptureStatus {
                    phase: LiveCapturePhase::Failed,
                    issue: Some(LiveCaptureIssue::RuntimeError),
                };
                true
            }
            CoreSignal::CaptureStopped => {
                self.controller
                    .lock()
                    .expect("live capture controller lock poisoned")
                    .capture_stopped();
                let mut status = self
                    .status
                    .lock()
                    .expect("live capture status lock poisoned");
                if status.phase != LiveCapturePhase::Failed {
                    *status = LiveCaptureStatus {
                        phase: LiveCapturePhase::Stopped,
                        issue: None,
                    };
                }
                true
            }
        };

        if affects_frontend_projection {
            self.bump_revision();
        }
    }
}

impl Drop for LiveCaptureInner {
    fn drop(&mut self) {
        self.controller
            .get_mut()
            .expect("live capture controller lock poisoned")
            .stop_if_running();
    }
}

fn engine_event_loop(inner: Weak<LiveCaptureInner>, receiver: Receiver<EngineEvent>) {
    while let Ok(event) = receiver.recv() {
        let Some(inner) = inner.upgrade() else {
            break;
        };
        inner.process_event(event);
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;
    use crate::engine::model::{Hit, HitCharacterSource, HitDirection, PacketObservation};

    fn hit(damage: f64) -> EngineEvent {
        EngineEvent::Hit(Box::new(Hit {
            timestamp: 1.0,
            char_id: 7,
            char_name: "Character 7".to_owned(),
            char_known: true,
            damage,
            byte_offset: 0,
            bit_shift: 0,
            char_source: HitCharacterSource::Packet,
            direction: HitDirection::Outgoing,
            target_hp_before: 0.0,
            target_hp_after: 0.0,
            target_max_hp: 0.0,
            target_hp_percent: 0.0,
            target_id: None,
            target_name: None,
            target_name_en: None,
            target_name_ja: None,
            target_monster_id: None,
            target_context: Vec::new(),
            gameplay_effect_index: None,
            gameplay_effect_name: None,
            ability_name: None,
            damage_name: None,
            damage_component: None,
            attack_type: None,
            damage_attribute: None,
            follow_up_damage: 0.0,
            follow_up_timestamp: None,
            follow_up_damage_name: None,
            follow_up_attack_type: None,
            follow_up_damage_attribute: None,
        }))
    }

    #[test]
    fn event_worker_routes_hits_through_the_shared_reducer() {
        let service = LiveCaptureService::new(LiveCaptureResources::default());
        let initial_revision = service.revision();
        service.ensure_event_worker().expect("event worker");
        service.0.sender.send(hit(321.0)).expect("test hit");

        let deadline = Instant::now() + Duration::from_secs(1);
        while service.with_state(|state| state.total_damage) != 321.0
            || service.revision() == initial_revision
        {
            assert!(Instant::now() < deadline, "event worker timed out");
            thread::yield_now();
        }

        assert_eq!(service.with_state(|state| state.hits.len()), 1);
        assert!(service.revision() > initial_revision);
    }

    #[test]
    fn runtime_failures_keep_private_details_out_of_status() {
        let service = LiveCaptureService::new(LiveCaptureResources::default());
        let initial_revision = service.revision();

        service
            .0
            .process_event(EngineEvent::Warning("private warning".to_owned()));
        assert_eq!(
            service.status().issue,
            Some(LiveCaptureIssue::RuntimeWarning)
        );
        let warning_revision = service.revision();
        assert!(warning_revision > initial_revision);

        service
            .0
            .process_event(EngineEvent::Error("private failure".to_owned()));
        assert_eq!(
            service.status(),
            LiveCaptureStatus {
                phase: LiveCapturePhase::Failed,
                issue: Some(LiveCaptureIssue::RuntimeError),
            }
        );
        assert!(service.revision() > warning_revision);
    }

    #[test]
    fn packet_quality_observations_do_not_invalidate_hud_projection() {
        let service = LiveCaptureService::new(LiveCaptureResources::default());
        let initial_revision = service.revision();

        service
            .0
            .process_event(EngineEvent::PacketObservation(PacketObservation {
                parsed_hits: 1,
            }));

        assert_eq!(service.revision(), initial_revision);
        assert_eq!(service.with_state(|state| state.packet_count), 1);
    }

    #[test]
    fn stopping_an_idle_service_reports_the_stable_core_error() {
        let service = LiveCaptureService::new(LiveCaptureResources::default());

        let error = service
            .request_stop()
            .expect_err("idle capture must reject stop");

        assert_eq!(error.code, CoreErrorCode::CaptureNotRunning);
        assert_eq!(service.status(), LiveCaptureStatus::default());
    }
}
