use std::sync::Mutex;

use nte_dps_tool::engine::model::{TeamDps, TeamDpsExport};

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ImportedTeams {
    pub(crate) upper: Option<TeamDps>,
    pub(crate) lower: Option<TeamDps>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TeamImportError {
    pub(crate) newly_unavailable: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TeamMutationOutcome {
    pub(crate) changed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TeamImportStatus {
    pub(crate) available: bool,
    pub(crate) upper_imported: bool,
    pub(crate) lower_imported: bool,
    pub(crate) newly_unavailable: bool,
}

#[derive(Clone, Debug, PartialEq)]
enum TeamImportState {
    Ready(ImportedTeams),
    Unavailable,
}

impl Default for TeamImportState {
    fn default() -> Self {
        Self::Ready(ImportedTeams::default())
    }
}

#[derive(Default)]
pub(crate) struct TeamImportService {
    state: Mutex<TeamImportState>,
}

impl TeamImportService {
    pub(crate) fn snapshot(
        &self,
        on_unavailable: impl Fn(),
    ) -> Result<ImportedTeams, TeamImportError> {
        match self.state.lock() {
            Ok(state) => match &*state {
                TeamImportState::Ready(teams) => Ok(teams.clone()),
                TeamImportState::Unavailable => Err(TeamImportError {
                    newly_unavailable: false,
                }),
            },
            Err(mut poison) => {
                **poison.get_mut() = TeamImportState::Unavailable;
                self.state.clear_poison();
                on_unavailable();
                log::warn!("Imported team state was closed after an interrupted update");
                Err(TeamImportError {
                    newly_unavailable: true,
                })
            }
        }
    }

    pub(crate) fn status(&self, on_unavailable: impl Fn()) -> TeamImportStatus {
        match self.snapshot(on_unavailable) {
            Ok(teams) => TeamImportStatus {
                available: true,
                upper_imported: teams.upper.is_some(),
                lower_imported: teams.lower.is_some(),
                newly_unavailable: false,
            },
            Err(error) => TeamImportStatus {
                available: false,
                upper_imported: false,
                lower_imported: false,
                newly_unavailable: error.newly_unavailable,
            },
        }
    }

    pub(crate) fn replace_export(
        &self,
        export: TeamDpsExport,
        on_change: impl Fn(),
    ) -> Result<TeamMutationOutcome, TeamImportError> {
        let fallback = export.single;
        let candidate = ImportedTeams {
            upper: export.upper.or_else(|| fallback.clone()),
            lower: export.lower.or(fallback),
        };
        self.replace(candidate, on_change)
    }

    pub(crate) fn replace_half(
        &self,
        upper: bool,
        team: TeamDps,
        on_change: impl Fn(),
    ) -> Result<TeamMutationOutcome, TeamImportError> {
        self.mutate(
            |teams| {
                let slot = if upper {
                    &mut teams.upper
                } else {
                    &mut teams.lower
                };
                if slot.as_ref() == Some(&team) {
                    return false;
                }
                *slot = Some(team);
                true
            },
            on_change,
        )
    }

    pub(crate) fn clear_half(
        &self,
        upper: bool,
        on_change: impl Fn(),
    ) -> Result<TeamMutationOutcome, TeamImportError> {
        self.mutate(
            |teams| {
                let slot = if upper {
                    &mut teams.upper
                } else {
                    &mut teams.lower
                };
                slot.take().is_some()
            },
            on_change,
        )
    }

    pub(crate) fn swap(
        &self,
        on_change: impl Fn(),
    ) -> Result<TeamMutationOutcome, TeamImportError> {
        self.mutate(
            |teams| {
                if teams.upper == teams.lower {
                    return false;
                }
                std::mem::swap(&mut teams.upper, &mut teams.lower);
                true
            },
            on_change,
        )
    }

    fn replace(
        &self,
        candidate: ImportedTeams,
        on_change: impl Fn(),
    ) -> Result<TeamMutationOutcome, TeamImportError> {
        self.mutate(
            |teams| {
                if *teams == candidate {
                    return false;
                }
                *teams = candidate;
                true
            },
            on_change,
        )
    }

    fn mutate(
        &self,
        update: impl FnOnce(&mut ImportedTeams) -> bool,
        on_change: impl Fn(),
    ) -> Result<TeamMutationOutcome, TeamImportError> {
        match self.state.lock() {
            Ok(mut state) => match &mut *state {
                TeamImportState::Ready(teams) => {
                    let changed = update(teams);
                    if changed {
                        on_change();
                    }
                    Ok(TeamMutationOutcome { changed })
                }
                TeamImportState::Unavailable => Err(TeamImportError {
                    newly_unavailable: false,
                }),
            },
            Err(mut poison) => {
                **poison.get_mut() = TeamImportState::Unavailable;
                self.state.clear_poison();
                on_change();
                Err(TeamImportError {
                    newly_unavailable: true,
                })
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn poison_for_test(&self) {
        let _ = std::panic::catch_unwind(|| {
            let _state = self.state.lock().expect("healthy team state");
            panic!("poison imported team state");
        });
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    fn team(dps: f64) -> TeamDps {
        TeamDps {
            dps,
            members: Vec::new(),
        }
    }

    #[test]
    fn whole_export_replace_is_atomic_and_noop_aware() {
        let service = TeamImportService::default();
        let publications = AtomicUsize::new(0);
        let export = TeamDpsExport {
            version: 1,
            single: None,
            upper: Some(team(10.0)),
            lower: Some(team(20.0)),
        };

        assert!(
            service
                .replace_export(export.clone(), || {
                    assert!(service.state.try_lock().is_err());
                    publications.fetch_add(1, Ordering::AcqRel);
                })
                .expect("replace")
                .changed
        );
        assert!(
            !service
                .replace_export(export, || {})
                .expect("same replace")
                .changed
        );
        assert_eq!(publications.load(Ordering::Acquire), 1);
        let snapshot = service.snapshot(|| {}).expect("snapshot");
        assert_eq!(snapshot.upper.expect("upper").dps, 10.0);
        assert_eq!(snapshot.lower.expect("lower").dps, 20.0);
    }

    #[test]
    fn clear_and_swap_report_only_observable_changes() {
        let service = TeamImportService::default();
        assert!(
            !service
                .clear_half(true, || {})
                .expect("clear empty")
                .changed
        );
        service
            .replace_half(true, team(10.0), || {})
            .expect("replace upper");
        assert!(service.swap(|| {}).expect("swap distinct halves").changed);
        assert!(service.swap(|| {}).expect("swap back").changed);
    }

    #[test]
    fn poisoned_team_state_never_projects_partial_data_as_empty() {
        let service = TeamImportService::default();
        service
            .replace_half(true, team(10.0), || {})
            .expect("replace upper");
        let _ = std::panic::catch_unwind(|| {
            let mut state = service.state.lock().expect("healthy team state");
            *state = TeamImportState::Ready(ImportedTeams {
                upper: None,
                lower: Some(team(99.0)),
            });
            panic!("interrupt two-half mutation");
        });

        let error = service
            .snapshot(|| {})
            .expect_err("poison must fail closed");
        assert!(error.newly_unavailable);
        let status = service.status(|| {});
        assert!(!status.available);
        assert!(!status.upper_imported);
        assert!(!status.lower_imported);
    }

    #[test]
    fn unavailable_team_state_does_not_overwrite_the_other_half() {
        let service = TeamImportService::default();
        let _ = std::panic::catch_unwind(|| {
            let _state = service.state.lock().expect("healthy team state");
            panic!("interrupt team mutation");
        });

        assert!(service.replace_half(false, team(30.0), || {}).is_err());
        assert!(service.snapshot(|| {}).is_err());
    }
}
