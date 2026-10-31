use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

use nte_dps_tool::core::character_data::{
    CharacterDataError, CharacterDataProjection, load_character_data,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CharacterDataServiceError {
    Busy,
    Domain(CharacterDataError),
}

pub(crate) struct CharacterDataService {
    path: PathBuf,
    busy: AtomicBool,
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
        }
    }

    pub(crate) fn snapshot(&self) -> Result<CharacterDataProjection, CharacterDataServiceError> {
        self.snapshot_with(load_character_data)
    }

    fn snapshot_with(
        &self,
        load: impl FnOnce(&Path) -> Result<CharacterDataProjection, CharacterDataError>,
    ) -> Result<CharacterDataProjection, CharacterDataServiceError> {
        let _permit = self.reserve()?;
        load(&self.path).map_err(CharacterDataServiceError::Domain)
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

    #[test]
    fn concurrent_operation_fails_fast_without_locking_file_work() {
        let service = CharacterDataService::new(PathBuf::from("private-character-path.json"));
        let projection = service
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
        assert!(!service.busy.load(Ordering::Acquire));
    }
}
