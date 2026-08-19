use std::{path::PathBuf, sync::Mutex};

use nte_dps_tool::core::encrypted_ini::{
    EncryptedIniDocument, EncryptedIniError, EncryptedIniKey, EncryptedIniSaveOutcome,
    load_encrypted_ini_document, save_encrypted_ini_document,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EncryptedIniProjection {
    pub(crate) generation: u64,
    pub(crate) display_path: Option<String>,
    pub(crate) file_name: Option<String>,
    pub(crate) key: EncryptedIniKey,
    pub(crate) plaintext: String,
    pub(crate) encrypted_line_count: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EncryptedIniServiceError {
    Busy,
    Unavailable,
    NoFile,
    StaleGeneration,
    Document(EncryptedIniError),
}

impl From<EncryptedIniError> for EncryptedIniServiceError {
    fn from(error: EncryptedIniError) -> Self {
        Self::Document(error)
    }
}

#[derive(Default)]
struct EncryptedIniRuntimeState {
    generation: u64,
    path: Option<PathBuf>,
    document: Option<EncryptedIniDocument>,
}

enum ServiceState {
    Ready(EncryptedIniRuntimeState),
    InFlight,
    Unavailable,
}

impl Default for ServiceState {
    fn default() -> Self {
        Self::Ready(EncryptedIniRuntimeState::default())
    }
}

#[derive(Default)]
pub(crate) struct EncryptedIniService {
    state: Mutex<ServiceState>,
}

struct EncryptedIniReservation<'a> {
    service: &'a EncryptedIniService,
    runtime: Option<EncryptedIniRuntimeState>,
    active: bool,
}

impl EncryptedIniService {
    pub(crate) fn snapshot(&self) -> Result<EncryptedIniProjection, EncryptedIniServiceError> {
        let state = match self.state.lock() {
            Ok(state) => state,
            Err(mut poison) => {
                **poison.get_mut() = ServiceState::Unavailable;
                self.state.clear_poison();
                return Err(EncryptedIniServiceError::Unavailable);
            }
        };
        match &*state {
            ServiceState::Ready(runtime) => Ok(project(runtime)),
            ServiceState::InFlight => Err(EncryptedIniServiceError::Busy),
            ServiceState::Unavailable => Err(EncryptedIniServiceError::Unavailable),
        }
    }

    pub(crate) fn open(
        &self,
        path: PathBuf,
    ) -> Result<EncryptedIniProjection, EncryptedIniServiceError> {
        self.open_with(path, load_encrypted_ini_document)
    }

    fn open_with(
        &self,
        path: PathBuf,
        load: impl FnOnce(&std::path::Path) -> Result<EncryptedIniDocument, EncryptedIniError>,
    ) -> Result<EncryptedIniProjection, EncryptedIniServiceError> {
        let mut reservation = self.reserve()?;
        // File validation, reading, and decryption run while the state mutex is
        // free. `InFlight` prevents a concurrent operation from seeing partial state.
        match load(&path) {
            Ok(document) => {
                let runtime = reservation.runtime_mut();
                runtime.generation = runtime.generation.wrapping_add(1);
                runtime.path = Some(path);
                runtime.document = Some(document);
                reservation.finish()
            }
            Err(error) => {
                reservation.finish()?;
                Err(EncryptedIniServiceError::Document(error))
            }
        }
    }

    pub(crate) fn reload(&self) -> Result<EncryptedIniProjection, EncryptedIniServiceError> {
        self.reload_with(load_encrypted_ini_document)
    }

    fn reload_with(
        &self,
        load: impl FnOnce(&std::path::Path) -> Result<EncryptedIniDocument, EncryptedIniError>,
    ) -> Result<EncryptedIniProjection, EncryptedIniServiceError> {
        let mut reservation = self.reserve()?;
        let Some(path) = reservation.runtime_mut().path.clone() else {
            reservation.finish()?;
            return Err(EncryptedIniServiceError::NoFile);
        };
        match load(&path) {
            Ok(document) => {
                let runtime = reservation.runtime_mut();
                runtime.generation = runtime.generation.wrapping_add(1);
                runtime.document = Some(document);
                reservation.finish()
            }
            Err(error) => {
                reservation.finish()?;
                Err(EncryptedIniServiceError::Document(error))
            }
        }
    }

    pub(crate) fn save(
        &self,
        expected_generation: u64,
        plaintext: String,
        key: EncryptedIniKey,
    ) -> Result<(EncryptedIniProjection, EncryptedIniSaveOutcome), EncryptedIniServiceError> {
        self.save_with(
            expected_generation,
            plaintext,
            key,
            save_encrypted_ini_document,
        )
    }

    fn save_with(
        &self,
        expected_generation: u64,
        plaintext: String,
        key: EncryptedIniKey,
        save: impl FnOnce(
            &std::path::Path,
            &mut EncryptedIniDocument,
            String,
            EncryptedIniKey,
        ) -> Result<EncryptedIniSaveOutcome, EncryptedIniError>,
    ) -> Result<(EncryptedIniProjection, EncryptedIniSaveOutcome), EncryptedIniServiceError> {
        let mut reservation = self.reserve()?;
        if reservation.runtime_mut().generation != expected_generation {
            reservation.finish()?;
            return Err(EncryptedIniServiceError::StaleGeneration);
        }
        let Some(path) = reservation.runtime_mut().path.clone() else {
            reservation.finish()?;
            return Err(EncryptedIniServiceError::NoFile);
        };
        let Some(document) = reservation.runtime_mut().document.as_mut() else {
            reservation.finish()?;
            return Err(EncryptedIniServiceError::NoFile);
        };
        // Encryption, serialization, and atomic persistence run with the
        // service mutex unlocked while this reservation owns the document.
        match save(&path, document, plaintext, key) {
            Ok(outcome) => {
                if outcome == EncryptedIniSaveOutcome::Saved {
                    let runtime = reservation.runtime_mut();
                    runtime.generation = runtime.generation.wrapping_add(1);
                }
                reservation.finish().map(|snapshot| (snapshot, outcome))
            }
            Err(error) => {
                reservation.finish()?;
                Err(EncryptedIniServiceError::Document(error))
            }
        }
    }

    pub(crate) fn clear(&self) -> Result<EncryptedIniProjection, EncryptedIniServiceError> {
        let mut reservation = self.reserve()?;
        let runtime = reservation.runtime_mut();
        if runtime.path.is_some() || runtime.document.is_some() {
            runtime.generation = runtime.generation.wrapping_add(1);
            runtime.path = None;
            runtime.document = None;
        }
        reservation.finish()
    }

    fn reserve(&self) -> Result<EncryptedIniReservation<'_>, EncryptedIniServiceError> {
        let mut state = match self.state.lock() {
            Ok(state) => state,
            Err(mut poison) => {
                **poison.get_mut() = ServiceState::Unavailable;
                self.state.clear_poison();
                return Err(EncryptedIniServiceError::Unavailable);
            }
        };
        let previous = std::mem::replace(&mut *state, ServiceState::InFlight);
        let runtime = match previous {
            ServiceState::Ready(runtime) => runtime,
            ServiceState::InFlight => {
                *state = ServiceState::InFlight;
                return Err(EncryptedIniServiceError::Busy);
            }
            ServiceState::Unavailable => {
                *state = ServiceState::Unavailable;
                return Err(EncryptedIniServiceError::Unavailable);
            }
        };
        Ok(EncryptedIniReservation {
            service: self,
            runtime: Some(runtime),
            active: true,
        })
    }

    #[cfg(test)]
    pub(crate) fn poison_for_test(&self) {
        let _ = std::panic::catch_unwind(|| {
            let _state = self.state.lock().expect("healthy encrypted INI state");
            panic!("poison encrypted INI state");
        });
    }
}

impl EncryptedIniReservation<'_> {
    fn runtime_mut(&mut self) -> &mut EncryptedIniRuntimeState {
        self.runtime
            .as_mut()
            .expect("active reservation owns encrypted INI runtime")
    }

    fn finish(mut self) -> Result<EncryptedIniProjection, EncryptedIniServiceError> {
        let runtime = self
            .runtime
            .take()
            .expect("active reservation owns encrypted INI runtime");
        let projection = project(&runtime);
        let mut state = match self.service.state.lock() {
            Ok(state) => state,
            Err(mut poison) => {
                **poison.get_mut() = ServiceState::Unavailable;
                self.service.state.clear_poison();
                self.active = false;
                return Err(EncryptedIniServiceError::Unavailable);
            }
        };
        if !matches!(*state, ServiceState::InFlight) {
            *state = ServiceState::Unavailable;
            self.active = false;
            return Err(EncryptedIniServiceError::Unavailable);
        }
        *state = ServiceState::Ready(runtime);
        self.active = false;
        Ok(projection)
    }
}

impl Drop for EncryptedIniReservation<'_> {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        self.runtime.take();
        match self.service.state.lock() {
            Ok(mut state) => *state = ServiceState::Unavailable,
            Err(mut poison) => {
                **poison.get_mut() = ServiceState::Unavailable;
                self.service.state.clear_poison();
            }
        }
    }
}

fn project(runtime: &EncryptedIniRuntimeState) -> EncryptedIniProjection {
    let document = runtime.document.as_ref();
    EncryptedIniProjection {
        generation: runtime.generation,
        display_path: runtime.path.as_ref().map(|path| path.display().to_string()),
        file_name: runtime
            .path
            .as_ref()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned()),
        key: document.map_or(EncryptedIniKey::Global, EncryptedIniDocument::key),
        plaintext: document
            .map(EncryptedIniDocument::plaintext)
            .unwrap_or_default()
            .to_owned(),
        encrypted_line_count: document.map_or(0, EncryptedIniDocument::encrypted_line_count),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        sync::atomic::{AtomicBool, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;

    struct Fixture {
        root: PathBuf,
        path: PathBuf,
    }

    impl Fixture {
        fn new(tag: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "nte-encrypted-service-{tag}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("clock after epoch")
                    .as_nanos()
            ));
            fs::create_dir_all(&root).expect("fixture directory");
            let path = root.join("Engine.ini");
            fs::write(&path, "Value=1\n").expect("fixture document");
            Self { root, path }
        }

        fn document(&self) -> EncryptedIniDocument {
            load_encrypted_ini_document(&self.path).expect("load fixture document")
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn file_work_runs_outside_the_state_mutex() {
        let fixture = Fixture::new("lock-free");
        let service = EncryptedIniService::default();
        let document = fixture.document();

        let opened = service
            .open_with(fixture.path.clone(), |_| {
                assert!(service.state.try_lock().is_ok());
                Ok(document)
            })
            .expect("open fixture");
        let (unchanged, outcome) = service
            .save_with(
                opened.generation,
                "Value=1".to_owned(),
                EncryptedIniKey::Global,
                |_, _, _, _| {
                    assert!(service.state.try_lock().is_ok());
                    Ok(EncryptedIniSaveOutcome::Unchanged)
                },
            )
            .expect("unchanged save");

        assert_eq!(opened.generation, 1);
        assert_eq!(unchanged.generation, 1);
        assert_eq!(outcome, EncryptedIniSaveOutcome::Unchanged);
    }

    #[test]
    fn saved_clear_and_noop_generations_are_exact() {
        let fixture = Fixture::new("generation");
        let service = EncryptedIniService::default();
        let opened = service
            .open_with(fixture.path.clone(), |_| Ok(fixture.document()))
            .expect("open fixture");
        let (saved, outcome) = service
            .save_with(
                opened.generation,
                "Value=2".to_owned(),
                EncryptedIniKey::Global,
                |_, _, _, _| Ok(EncryptedIniSaveOutcome::Saved),
            )
            .expect("saved update");
        let cleared = service.clear().expect("clear loaded document");
        let repeated = service.clear().expect("repeat clear");

        assert_eq!(outcome, EncryptedIniSaveOutcome::Saved);
        assert_eq!(saved.generation, 2);
        assert_eq!(cleared.generation, 3);
        assert_eq!(repeated.generation, 3);
    }

    #[test]
    fn stale_and_failed_operations_do_not_run_or_bump() {
        let fixture = Fixture::new("failure");
        let service = EncryptedIniService::default();
        let opened = service
            .open_with(fixture.path.clone(), |_| Ok(fixture.document()))
            .expect("open fixture");
        let called = AtomicBool::new(false);

        let stale = service.save_with(0, String::new(), EncryptedIniKey::Global, |_, _, _, _| {
            called.store(true, Ordering::Release);
            Ok(EncryptedIniSaveOutcome::Saved)
        });
        let failed = service.reload_with(|_| Err(EncryptedIniError::ReadFailed));
        let snapshot = service.snapshot().expect("state preserved");

        assert_eq!(stale, Err(EncryptedIniServiceError::StaleGeneration));
        assert_eq!(
            failed,
            Err(EncryptedIniServiceError::Document(
                EncryptedIniError::ReadFailed
            ))
        );
        assert!(!called.load(Ordering::Acquire));
        assert_eq!(snapshot.generation, opened.generation);
        assert_eq!(snapshot.plaintext, opened.plaintext);
    }

    #[test]
    fn poisoned_state_is_sticky_and_never_runs_file_work() {
        let service = EncryptedIniService::default();
        service.poison_for_test();
        let called = AtomicBool::new(false);

        let first = service.open_with(PathBuf::from("private.ini"), |_| {
            called.store(true, Ordering::Release);
            Err(EncryptedIniError::ReadFailed)
        });
        let second = service.snapshot();

        assert_eq!(first, Err(EncryptedIniServiceError::Unavailable));
        assert_eq!(second, Err(EncryptedIniServiceError::Unavailable));
        assert!(!called.load(Ordering::Acquire));
    }
}
