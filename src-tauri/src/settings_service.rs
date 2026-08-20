use std::{
    fmt,
    path::{Path, PathBuf},
    sync::{
        Mutex, MutexGuard,
        atomic::{AtomicU64, Ordering},
    },
};

use nte_dps_tool::{
    core::{CoreError, capture::enumerate_devices},
    storage::config::{self, UiConfig},
};

use crate::contract::settings::CaptureDeviceSnapshot;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SettingsServiceError {
    TransactionUnavailable,
    ConfigSave,
    PassthroughUnavailable,
}

impl fmt::Display for SettingsServiceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::TransactionUnavailable => "settings transaction unavailable",
            Self::ConfigSave => "settings save failed",
            Self::PassthroughUnavailable => "passthrough transaction unavailable",
        })
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ConfigUpdate {
    pub(crate) previous: UiConfig,
    pub(crate) current: UiConfig,
}

#[derive(Clone, Debug)]
pub(crate) struct DeviceCatalogSnapshot {
    pub(crate) devices: Vec<CaptureDeviceSnapshot>,
    pub(crate) available: bool,
}

#[derive(Debug)]
pub(crate) struct DeviceRefreshOutcome {
    pub(crate) error: Option<CoreError>,
}

#[derive(Clone, Debug)]
struct DeviceCatalogState {
    devices: Vec<CaptureDeviceSnapshot>,
    available: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TransactionState {
    Ready,
    Unavailable,
}

pub(crate) struct SettingsService {
    config_path: PathBuf,
    config: Mutex<UiConfig>,
    config_transaction: Mutex<TransactionState>,
    devices: Mutex<DeviceCatalogState>,
    device_probe_generation: AtomicU64,
    passthrough_transaction: Mutex<TransactionState>,
}

pub(crate) struct PassthroughTransactionGuard<'a> {
    _guard: MutexGuard<'a, TransactionState>,
}

impl SettingsService {
    pub(crate) fn new(
        config: UiConfig,
        config_path: PathBuf,
        devices: Result<Vec<CaptureDeviceSnapshot>, CoreError>,
    ) -> Self {
        let (devices, available) = match devices {
            Ok(devices) => (devices, true),
            Err(_) => (Vec::new(), false),
        };
        Self {
            config_path,
            config: Mutex::new(config.sanitized()),
            config_transaction: Mutex::new(TransactionState::Ready),
            devices: Mutex::new(DeviceCatalogState { devices, available }),
            device_probe_generation: AtomicU64::new(0),
            passthrough_transaction: Mutex::new(TransactionState::Ready),
        }
    }

    pub(crate) fn config_snapshot(&self) -> UiConfig {
        match self.config.lock() {
            Ok(config) => config.clone(),
            Err(mut poison) => {
                // `config` never exposes a mutable guard. Updates build a complete,
                // sanitized candidate without this lock and only replace the whole
                // value while locked, so a panic cannot expose a partial mutation.
                let recovered = poison.get_mut().clone().sanitized();
                **poison.get_mut() = recovered.clone();
                self.config.clear_poison();
                log::warn!("Settings snapshot recovered a complete committed configuration");
                recovered
            }
        }
    }

    pub(crate) fn update_config(
        &self,
        update: impl FnOnce(&mut UiConfig),
        on_commit: impl FnOnce(&ConfigUpdate),
    ) -> Result<Option<ConfigUpdate>, SettingsServiceError> {
        self.update_config_with(update, config::save, on_commit)
    }

    fn update_config_with(
        &self,
        update: impl FnOnce(&mut UiConfig),
        save: impl FnOnce(&Path, &UiConfig) -> Result<(), String>,
        on_commit: impl FnOnce(&ConfigUpdate),
    ) -> Result<Option<ConfigUpdate>, SettingsServiceError> {
        let transaction = match self.config_transaction.lock() {
            Ok(transaction) => transaction,
            Err(mut poison) => {
                **poison.get_mut() = TransactionState::Unavailable;
                self.config_transaction.clear_poison();
                return Err(SettingsServiceError::TransactionUnavailable);
            }
        };
        if *transaction == TransactionState::Unavailable {
            return Err(SettingsServiceError::TransactionUnavailable);
        }

        let previous = self.config_snapshot();
        let mut current = previous.clone();
        update(&mut current);
        current = current.sanitized();
        if current == previous {
            return Ok(None);
        }

        // The transaction gate deliberately serializes config file commits, but
        // no config/device/team state lock is held across this disk I/O.
        save(&self.config_path, &current).map_err(|_| SettingsServiceError::ConfigSave)?;
        let mut config = match self.config.lock() {
            Ok(config) => config,
            Err(mut poison) => {
                // See `config_snapshot`: only whole validated values are stored.
                **poison.get_mut() = current.clone();
                self.config.clear_poison();
                poison.into_inner()
            }
        };
        *config = current.clone();
        let committed = ConfigUpdate { previous, current };
        // Publish the revision while the committed value is still locked. A
        // snapshot that observed the old revision must therefore either read
        // the old value or notice the new revision and retry.
        on_commit(&committed);
        drop(config);
        drop(transaction);
        Ok(Some(committed))
    }

    pub(crate) fn device_catalog_snapshot(&self, on_recovery: impl Fn()) -> DeviceCatalogSnapshot {
        match self.devices.lock() {
            Ok(devices) => DeviceCatalogSnapshot {
                devices: devices.devices.clone(),
                available: devices.available,
            },
            Err(mut poison) => {
                // Device state is rebuildable. Never project possibly partial
                // entries; the next successful OS enumeration replaces all data.
                **poison.get_mut() = DeviceCatalogState {
                    devices: Vec::new(),
                    available: false,
                };
                self.devices.clear_poison();
                on_recovery();
                log::warn!("Capture device catalog was discarded after an interrupted update");
                DeviceCatalogSnapshot {
                    devices: Vec::new(),
                    available: false,
                }
            }
        }
    }

    pub(crate) fn refresh_devices(&self, on_change: impl Fn()) -> DeviceRefreshOutcome {
        self.refresh_devices_with(
            || {
                enumerate_devices()
                    .map(|devices| devices.iter().map(CaptureDeviceSnapshot::from).collect())
            },
            on_change,
        )
    }

    pub(crate) fn refresh_devices_with(
        &self,
        enumerate: impl FnOnce() -> Result<Vec<CaptureDeviceSnapshot>, CoreError>,
        on_change: impl Fn(),
    ) -> DeviceRefreshOutcome {
        let generation = self
            .device_probe_generation
            .fetch_add(1, Ordering::AcqRel)
            .wrapping_add(1);
        // Enumeration is an OS/FFI boundary and must complete without the catalog lock.
        let candidate = enumerate();
        let devices = self.devices.lock();
        if generation != self.device_probe_generation.load(Ordering::Acquire) {
            // A newer request owns publication. Stale success and failure both
            // leave the latest whole catalog and its availability untouched.
            drop(devices);
            return DeviceRefreshOutcome { error: None };
        }
        match candidate {
            Ok(candidate) => match devices {
                Ok(mut devices) => {
                    let changed = !devices.available || devices.devices != candidate;
                    *devices = DeviceCatalogState {
                        devices: candidate,
                        available: true,
                    };
                    if changed {
                        on_change();
                    }
                    DeviceRefreshOutcome { error: None }
                }
                Err(mut poison) => {
                    **poison.get_mut() = DeviceCatalogState {
                        devices: candidate,
                        available: true,
                    };
                    self.devices.clear_poison();
                    on_change();
                    DeviceRefreshOutcome { error: None }
                }
            },
            Err(error) => {
                match devices {
                    Ok(mut devices) => {
                        let changed = devices.available;
                        devices.available = false;
                        if changed {
                            on_change();
                        }
                    }
                    Err(mut poison) => {
                        **poison.get_mut() = DeviceCatalogState {
                            devices: Vec::new(),
                            available: false,
                        };
                        self.devices.clear_poison();
                        on_change();
                    }
                };
                DeviceRefreshOutcome { error: Some(error) }
            }
        }
    }

    pub(crate) fn lock_passthrough_transaction(
        &self,
    ) -> Result<PassthroughTransactionGuard<'_>, SettingsServiceError> {
        let guard = match self.passthrough_transaction.lock() {
            Ok(guard) => guard,
            Err(mut poison) => {
                **poison.get_mut() = TransactionState::Unavailable;
                self.passthrough_transaction.clear_poison();
                return Err(SettingsServiceError::PassthroughUnavailable);
            }
        };
        if *guard == TransactionState::Unavailable {
            return Err(SettingsServiceError::PassthroughUnavailable);
        }
        Ok(PassthroughTransactionGuard { _guard: guard })
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
            mpsc,
        },
        thread,
    };

    use nte_dps_tool::core::CoreErrorCode;

    use super::*;

    fn device(id: &str) -> CaptureDeviceSnapshot {
        CaptureDeviceSnapshot {
            id: id.to_owned(),
            label: id.to_owned(),
        }
    }

    fn service_with_devices(devices: Vec<CaptureDeviceSnapshot>) -> SettingsService {
        SettingsService::new(
            UiConfig::default(),
            PathBuf::from("private-config.json"),
            Ok(devices),
        )
    }

    #[test]
    fn equal_config_update_does_not_call_the_saver() {
        let service = service_with_devices(Vec::new());
        let saves = AtomicUsize::new(0);

        let outcome = service
            .update_config_with(
                |_| {},
                |_, _| {
                    saves.fetch_add(1, Ordering::AcqRel);
                    Ok(())
                },
                |_| {},
            )
            .expect("healthy update");

        assert!(outcome.is_none());
        assert_eq!(saves.load(Ordering::Acquire), 0);
    }

    #[test]
    fn config_commit_publishes_while_the_committed_generation_is_locked() {
        let service = service_with_devices(Vec::new());
        let publications = AtomicUsize::new(0);

        let outcome = service
            .update_config_with(
                |config| config.opacity = 0.42,
                |_, _| Ok(()),
                |change| {
                    assert!(service.config.try_lock().is_err());
                    assert_ne!(change.previous.opacity, change.current.opacity);
                    publications.fetch_add(1, Ordering::AcqRel);
                },
            )
            .expect("commit configuration")
            .expect("observable configuration change");

        assert_eq!(outcome.current.opacity, 0.42);
        assert_eq!(publications.load(Ordering::Acquire), 1);
    }

    #[test]
    fn poisoned_config_transaction_blocks_followup_disk_writes() {
        let service = service_with_devices(Vec::new());
        let _ = std::panic::catch_unwind(|| {
            let _guard = service
                .config_transaction
                .lock()
                .expect("healthy transaction");
            panic!("interrupt settings transaction before its commit point");
        });
        let saves = AtomicUsize::new(0);

        let result = service.update_config_with(
            |config| config.opacity = 0.42,
            |_, _| {
                saves.fetch_add(1, Ordering::AcqRel);
                Ok(())
            },
            |_| {},
        );

        assert!(matches!(
            result,
            Err(SettingsServiceError::TransactionUnavailable)
        ));
        assert_eq!(saves.load(Ordering::Acquire), 0);
        assert_eq!(
            service.config_snapshot().opacity,
            UiConfig::default().opacity
        );
    }

    #[test]
    fn device_probe_runs_without_the_device_lock_and_preserves_last_good_as_degraded() {
        let service = service_with_devices(vec![device("last-good")]);
        let changes = AtomicUsize::new(0);
        let outcome = service.refresh_devices_with(
            || {
                assert!(service.devices.try_lock().is_ok());
                Err(CoreError::new(
                    CoreErrorCode::SystemProbeFailed,
                    "private adapter path",
                ))
            },
            || {
                assert!(service.devices.try_lock().is_err());
                changes.fetch_add(1, Ordering::AcqRel);
            },
        );

        assert!(outcome.error.is_some());
        assert_eq!(changes.load(Ordering::Acquire), 1);
        let snapshot = service.device_catalog_snapshot(|| {});
        assert!(!snapshot.available);
        assert_eq!(snapshot.devices[0].id, "last-good");
    }

    #[test]
    fn poisoned_device_catalog_is_not_projected_as_a_healthy_empty_list() {
        let service = service_with_devices(vec![device("private-device")]);
        let _ = std::panic::catch_unwind(|| {
            let mut devices = service.devices.lock().expect("healthy device catalog");
            devices.devices.clear();
            panic!("interrupt device catalog mutation");
        });

        let recoveries = AtomicUsize::new(0);
        let snapshot = service.device_catalog_snapshot(|| {
            recoveries.fetch_add(1, Ordering::AcqRel);
        });
        assert!(!snapshot.available);
        assert_eq!(recoveries.load(Ordering::Acquire), 1);
    }

    #[test]
    fn poisoned_passthrough_gate_fails_closed_without_running_the_operation() {
        let service = service_with_devices(Vec::new());
        let _ = std::panic::catch_unwind(|| {
            let _guard = service
                .passthrough_transaction
                .lock()
                .expect("healthy passthrough transaction");
            panic!("interrupt passthrough transition");
        });

        assert!(matches!(
            service.lock_passthrough_transaction(),
            Err(SettingsServiceError::PassthroughUnavailable)
        ));
    }

    #[test]
    fn slow_old_success_cannot_overwrite_fast_new_success() {
        let service = Arc::new(service_with_devices(vec![device("initial")]));
        let publications = Arc::new(AtomicUsize::new(0));
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let old_service = Arc::clone(&service);
        let old_publications = Arc::clone(&publications);
        let old = thread::spawn(move || {
            old_service.refresh_devices_with(
                || {
                    started_tx.send(()).expect("announce old probe");
                    release_rx.recv().expect("release old probe");
                    Ok(vec![device("old")])
                },
                || {
                    old_publications.fetch_add(1, Ordering::AcqRel);
                },
            )
        });
        started_rx.recv().expect("old probe started");

        service.refresh_devices_with(
            || Ok(vec![device("new")]),
            || {
                publications.fetch_add(1, Ordering::AcqRel);
            },
        );
        release_tx.send(()).expect("release old probe");
        let old_outcome = old.join().expect("join old probe");

        assert!(old_outcome.error.is_none());
        assert_eq!(publications.load(Ordering::Acquire), 1);
        assert_eq!(
            service.device_catalog_snapshot(|| {}).devices,
            vec![device("new")]
        );
    }

    #[test]
    fn slow_old_failure_cannot_degrade_fast_new_success() {
        let service = Arc::new(service_with_devices(vec![device("initial")]));
        let publications = Arc::new(AtomicUsize::new(0));
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let old_service = Arc::clone(&service);
        let old_publications = Arc::clone(&publications);
        let old = thread::spawn(move || {
            old_service.refresh_devices_with(
                || {
                    started_tx.send(()).expect("announce old probe");
                    release_rx.recv().expect("release old probe");
                    Err(CoreError::new(
                        CoreErrorCode::SystemProbeFailed,
                        "private old probe detail",
                    ))
                },
                || {
                    old_publications.fetch_add(1, Ordering::AcqRel);
                },
            )
        });
        started_rx.recv().expect("old probe started");

        service.refresh_devices_with(
            || Ok(vec![device("new")]),
            || {
                publications.fetch_add(1, Ordering::AcqRel);
            },
        );
        release_tx.send(()).expect("release old probe");
        let old_outcome = old.join().expect("join old probe");
        let snapshot = service.device_catalog_snapshot(|| {});

        assert!(old_outcome.error.is_none());
        assert!(snapshot.available);
        assert_eq!(snapshot.devices, vec![device("new")]);
        assert_eq!(publications.load(Ordering::Acquire), 1);
    }

    #[test]
    fn latest_success_rebuilds_poison_while_an_old_probe_stays_stale() {
        let service = Arc::new(service_with_devices(vec![device("initial")]));
        let _ = std::panic::catch_unwind({
            let service = Arc::clone(&service);
            move || {
                let _devices = service.devices.lock().expect("healthy catalog");
                panic!("poison device catalog before overlapping probes");
            }
        });
        let publications = Arc::new(AtomicUsize::new(0));
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let old_service = Arc::clone(&service);
        let old_publications = Arc::clone(&publications);
        let old = thread::spawn(move || {
            old_service.refresh_devices_with(
                || {
                    started_tx.send(()).expect("announce old probe");
                    release_rx.recv().expect("release old probe");
                    Ok(vec![device("old")])
                },
                || {
                    old_publications.fetch_add(1, Ordering::AcqRel);
                },
            )
        });
        started_rx.recv().expect("old probe started");

        service.refresh_devices_with(
            || Ok(vec![device("rebuilt")]),
            || {
                publications.fetch_add(1, Ordering::AcqRel);
            },
        );
        release_tx.send(()).expect("release old probe");
        old.join().expect("join old probe");
        let snapshot = service.device_catalog_snapshot(|| {});

        assert!(snapshot.available);
        assert_eq!(snapshot.devices, vec![device("rebuilt")]);
        assert_eq!(publications.load(Ordering::Acquire), 1);
    }
}
