use super::*;

impl AppState {
    pub(crate) fn import_team_data(&self, export: TeamDpsExport) -> Result<bool, TeamImportError> {
        let outcome = self.0.team_import.replace_export(export, || {
            self.publish_settings_effects(SettingsMutationEffects::SETTINGS);
        })?;
        Ok(outcome.changed)
    }

    pub(crate) fn imported_abyss_teams(
        &self,
    ) -> Result<(Option<TeamDps>, Option<TeamDps>), TeamImportError> {
        self.0
            .team_import
            .snapshot(|| {
                self.publish_settings_effects(SettingsMutationEffects::SETTINGS);
            })
            .map(|teams| (teams.upper, teams.lower))
    }

    pub(crate) fn import_abyss_team(
        &self,
        export: TeamDpsExport,
        upper: bool,
    ) -> Result<bool, TeamImportError> {
        let preferred = if upper { export.upper } else { export.lower };
        let Some(team) = preferred.or(export.single) else {
            return Ok(false);
        };
        let _outcome = self.0.team_import.replace_half(upper, team, || {
            self.publish_settings_effects(SettingsMutationEffects::SETTINGS);
        })?;
        Ok(true)
    }

    pub(crate) fn import_current_abyss_team(
        &self,
        upper: bool,
    ) -> Result<bool, TeamOperationError> {
        let Some(team) = self.current_abyss_team(upper)? else {
            return Ok(false);
        };
        let _outcome = self
            .0
            .team_import
            .replace_half(upper, team, || {
                self.publish_settings_effects(SettingsMutationEffects::SETTINGS);
            })
            .map_err(TeamOperationError::State)?;
        Ok(true)
    }

    pub(crate) fn current_abyss_team_availability(&self) -> Result<[bool; 2], CoreError> {
        Ok([
            self.current_abyss_team(true)?.is_some(),
            self.current_abyss_team(false)?.is_some(),
        ])
    }

    fn current_abyss_team(&self, upper: bool) -> Result<Option<TeamDps>, CoreError> {
        let config = self.ui_config();
        let dps_time_mode = self.main_presented_dps_time_mode()?;
        self.with_main_presented_state(|state| {
            let export = nte_dps_tool::core::team_data::export_team_data(
                state,
                subtract_time_stop_for_state(dps_time_mode, state),
                config.separate_reaction_damage,
                None,
                None,
            )?;
            if state.abyss.is_active() {
                if upper { export.upper } else { export.lower }
            } else {
                export.single
            }
        })
    }

    pub(crate) fn clear_abyss_team(&self, upper: bool) -> Result<bool, TeamImportError> {
        let outcome = self.0.team_import.clear_half(upper, || {
            self.publish_settings_effects(SettingsMutationEffects::SETTINGS);
        })?;
        Ok(outcome.changed)
    }

    pub(crate) fn swap_abyss_teams(&self) -> Result<bool, TeamImportError> {
        let outcome = self.0.team_import.swap(|| {
            self.publish_settings_effects(SettingsMutationEffects::SETTINGS);
        })?;
        Ok(outcome.changed)
    }

    pub(crate) fn export_team_data(&self) -> Result<Option<TeamDpsExport>, TeamOperationError> {
        let config = self.ui_config();
        let dps_time_mode = self.main_presented_dps_time_mode()?;
        let imported = self
            .0
            .team_import
            .snapshot(|| {
                self.publish_settings_effects(SettingsMutationEffects::SETTINGS);
            })
            .map_err(TeamOperationError::State)?;
        Ok(self.with_main_presented_state(|state| {
            nte_dps_tool::core::team_data::export_team_data(
                state,
                subtract_time_stop_for_state(dps_time_mode, state),
                config.separate_reaction_damage,
                imported.upper,
                imported.lower,
            )
        })?)
    }
}
