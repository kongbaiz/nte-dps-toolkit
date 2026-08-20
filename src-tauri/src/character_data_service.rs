use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};

use nte_dps_tool::core::character_data::{
    CharacterDataError, CharacterDataProjection, CharacterDataRecordInput,
    CharacterDataSaveOutcome, load_character_data, save_character_data_record,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CharacterDataServiceError {
    Busy,
    Domain(CharacterDataError),
}

impl From<CharacterDataError> for CharacterDataServiceError {
    fn from(error: CharacterDataError) -> Self {
        Self::Domain(error)
    }
}

pub(crate) struct CharacterDataService {
    path: PathBuf,
    busy: AtomicBool,
    revision: AtomicU64,
}

struct CharacterDataPermit<'a>(&'a AtomicBool);

impl Drop for CharacterDataPermit<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

impl CharacterDataService {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self {
            path,
            busy: AtomicBool::new(false),
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
        let _permit = self.reserve()?;
        let projection = load(&self.path).map_err(CharacterDataServiceError::Domain)?;
        Ok((projection, self.revision.load(Ordering::Acquire)))
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
        let _permit = self.reserve()?;
        let (projection, outcome) =
            save(&self.path, input).map_err(CharacterDataServiceError::Domain)?;
        let revision = if outcome == CharacterDataSaveOutcome::Saved {
            self.revision
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                    Some(current.saturating_add(1))
                })
                .map_or(u64::MAX, |previous| previous.saturating_add(1))
        } else {
            self.revision.load(Ordering::Acquire)
        };
        Ok((projection, revision))
    }

    fn reserve(&self) -> Result<CharacterDataPermit<'_>, CharacterDataServiceError> {
        self.busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| CharacterDataServiceError::Busy)?;
        Ok(CharacterDataPermit(&self.busy))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> CharacterDataRecordInput {
        CharacterDataRecordInput {
            id: "1".to_owned(),
            name_en: "Fixture".to_owned(),
            ..CharacterDataRecordInput::default()
        }
    }

    #[test]
    fn concurrent_operation_fails_fast_without_locking_file_work() {
        let service = CharacterDataService::new(PathBuf::from("private-character-path.json"));
        let (projection, revision) = service
            .snapshot_with(|_| {
                assert!(service.busy.load(Ordering::Acquire));
                assert_eq!(
                    service.snapshot_with(|_| Ok(CharacterDataProjection::default())),
                    Err(CharacterDataServiceError::Busy)
                );
                Ok(CharacterDataProjection::default())
            })
            .expect("load projection");

        assert!(projection.records.is_empty());
        assert_eq!(revision, 0);
        assert!(!service.busy.load(Ordering::Acquire));
    }

    #[test]
    fn saved_changes_bump_once_and_noops_do_not_bump() {
        let service = CharacterDataService::new(PathBuf::from("private-character-path.json"));
        let (_, saved_revision) = service
            .save_record_with(input(), |_, _| {
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
    fn failed_save_does_not_bump_and_releases_the_gate() {
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
}
