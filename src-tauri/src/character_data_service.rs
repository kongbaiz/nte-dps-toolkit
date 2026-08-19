use std::{
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use nte_dps_tool::core::character_data::{
    CharacterDataError, CharacterDataProjection, CharacterDataRecordInput,
    CharacterDataSaveOutcome, load_character_data, save_character_data_record,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CharacterDataServiceError {
    Busy,
    Unavailable,
    Domain(CharacterDataError),
}

impl From<CharacterDataError> for CharacterDataServiceError {
    fn from(error: CharacterDataError) -> Self {
        Self::Domain(error)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum TransactionState {
    #[default]
    Ready,
    Busy,
    Unavailable,
}

pub(crate) struct CharacterDataService {
    path: PathBuf,
    transaction: Mutex<TransactionState>,
    revision: AtomicU64,
}

struct CharacterDataReservation<'a> {
    service: &'a CharacterDataService,
    active: bool,
}

impl CharacterDataService {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self {
            path,
            transaction: Mutex::new(TransactionState::Ready),
            revision: AtomicU64::new(0),
        }
    }

    pub(crate) fn snapshot(
        &self,
    ) -> Result<(CharacterDataProjection, u64), CharacterDataServiceError> {
        self.snapshot_with(load_character_data)
    }

    fn snapshot_with(
        &self,
        load: impl FnOnce(&Path) -> Result<CharacterDataProjection, CharacterDataError>,
    ) -> Result<(CharacterDataProjection, u64), CharacterDataServiceError> {
        let reservation = self.reserve()?;
        // The transaction mutex records ownership only. File I/O and JSON
        // parsing run after its guard has been released.
        let projection = load(&self.path);
        let revision = reservation.finish(|| self.revision.load(Ordering::Acquire))?;
        let projection = projection.map_err(CharacterDataServiceError::Domain)?;
        Ok((projection, revision))
    }

    pub(crate) fn save_record(
        &self,
        input: CharacterDataRecordInput,
    ) -> Result<(CharacterDataProjection, u64), CharacterDataServiceError> {
        self.save_record_with(input, save_character_data_record)
    }

    fn save_record_with(
        &self,
        input: CharacterDataRecordInput,
        save: impl FnOnce(
            &Path,
            CharacterDataRecordInput,
        ) -> Result<
            (CharacterDataProjection, CharacterDataSaveOutcome),
            CharacterDataError,
        >,
    ) -> Result<(CharacterDataProjection, u64), CharacterDataServiceError> {
        let reservation = self.reserve()?;
        // Reading, validation, serialization, and atomic persistence all run
        // without holding the transaction mutex.
        let result = save(&self.path, input);
        let saved = matches!(result, Ok((_, CharacterDataSaveOutcome::Saved)));
        let revision = reservation.finish(|| {
            if saved {
                self.revision.fetch_add(1, Ordering::AcqRel);
            }
            self.revision.load(Ordering::Acquire)
        })?;
        let (projection, _) = result.map_err(CharacterDataServiceError::Domain)?;
        Ok((projection, revision))
    }

    fn reserve(&self) -> Result<CharacterDataReservation<'_>, CharacterDataServiceError> {
        let mut transaction = match self.transaction.lock() {
            Ok(transaction) => transaction,
            Err(mut poison) => {
                **poison.get_mut() = TransactionState::Unavailable;
                self.transaction.clear_poison();
                return Err(CharacterDataServiceError::Unavailable);
            }
        };
        match *transaction {
            TransactionState::Ready => *transaction = TransactionState::Busy,
            TransactionState::Busy => return Err(CharacterDataServiceError::Busy),
            TransactionState::Unavailable => {
                return Err(CharacterDataServiceError::Unavailable);
            }
        }
        Ok(CharacterDataReservation {
            service: self,
            active: true,
        })
    }

    #[cfg(test)]
    pub(crate) fn poison_for_test(&self) {
        let _ = std::panic::catch_unwind(|| {
            let _transaction = self.transaction.lock().expect("healthy transaction");
            panic!("poison character-data transaction");
        });
    }
}

impl CharacterDataReservation<'_> {
    fn finish<T>(mut self, publish: impl FnOnce() -> T) -> Result<T, CharacterDataServiceError> {
        let mut transaction = match self.service.transaction.lock() {
            Ok(transaction) => transaction,
            Err(mut poison) => {
                **poison.get_mut() = TransactionState::Unavailable;
                self.service.transaction.clear_poison();
                self.active = false;
                return Err(CharacterDataServiceError::Unavailable);
            }
        };
        if *transaction != TransactionState::Busy {
            *transaction = TransactionState::Unavailable;
            self.active = false;
            return Err(CharacterDataServiceError::Unavailable);
        }
        let published = publish();
        *transaction = TransactionState::Ready;
        self.active = false;
        Ok(published)
    }
}

impl Drop for CharacterDataReservation<'_> {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        match self.service.transaction.lock() {
            Ok(mut transaction) => *transaction = TransactionState::Unavailable,
            Err(mut poison) => {
                **poison.get_mut() = TransactionState::Unavailable;
                self.service.transaction.clear_poison();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;

    fn input() -> CharacterDataRecordInput {
        CharacterDataRecordInput {
            id: "1".to_owned(),
            name_en: "Fixture".to_owned(),
            ..CharacterDataRecordInput::default()
        }
    }

    #[test]
    fn file_work_runs_without_the_transaction_mutex() {
        let service = CharacterDataService::new(PathBuf::from("private-character-path.json"));

        let (projection, revision) = service
            .snapshot_with(|_| {
                let state = service
                    .transaction
                    .try_lock()
                    .expect("transaction mutex free");
                assert_eq!(*state, TransactionState::Busy);
                Ok(CharacterDataProjection::default())
            })
            .expect("load projection");

        assert!(projection.records.is_empty());
        assert_eq!(revision, 0);
    }

    #[test]
    fn saved_changes_bump_once_and_noops_do_not_bump() {
        let service = CharacterDataService::new(PathBuf::from("private-character-path.json"));

        let (_, saved_revision) = service
            .save_record_with(input(), |_, _| {
                assert!(service.transaction.try_lock().is_ok());
                Ok((
                    CharacterDataProjection::default(),
                    CharacterDataSaveOutcome::Saved,
                ))
            })
            .expect("saved update");
        let (_, unchanged_revision) = service
            .save_record_with(input(), |_, _| {
                Ok((
                    CharacterDataProjection::default(),
                    CharacterDataSaveOutcome::Unchanged,
                ))
            })
            .expect("unchanged update");

        assert_eq!(saved_revision, 1);
        assert_eq!(unchanged_revision, 1);
    }

    #[test]
    fn failed_save_does_not_bump_and_the_service_remains_usable() {
        let service = CharacterDataService::new(PathBuf::from("private-character-path.json"));

        let result = service.save_record_with(input(), |_, _| {
            Err(CharacterDataError::Write("private path detail".to_owned()))
        });

        assert!(matches!(
            result,
            Err(CharacterDataServiceError::Domain(
                CharacterDataError::Write(_)
            ))
        ));
        assert_eq!(service.revision.load(Ordering::Acquire), 0);
        assert!(
            service
                .snapshot_with(|_| Ok(CharacterDataProjection::default()))
                .is_ok()
        );
    }

    #[test]
    fn poisoned_transaction_is_sticky_and_blocks_file_work() {
        let service = CharacterDataService::new(PathBuf::from("private-character-path.json"));
        service.poison_for_test();
        let called = AtomicBool::new(false);

        let first = service.snapshot_with(|_| {
            called.store(true, Ordering::Release);
            Ok(CharacterDataProjection::default())
        });
        let second = service.snapshot_with(|_| {
            called.store(true, Ordering::Release);
            Ok(CharacterDataProjection::default())
        });

        assert_eq!(first, Err(CharacterDataServiceError::Unavailable));
        assert_eq!(second, Err(CharacterDataServiceError::Unavailable));
        assert!(!called.load(Ordering::Acquire));
    }
}
