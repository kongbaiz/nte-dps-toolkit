use crate::{
    contract::settings::UpdateSettingsSnapshot,
    settings_service::{ConfigUpdate, PassthroughTransactionGuard},
    update_runtime::UpdateRuntimeSnapshot,
};
use nte_dps_tool::storage::{
    capture_logs::{ClearOutcome, clear_capture_logs, scan_capture_logs},
    config::GlobalHotkeys,
};

use super::*;

impl AppState {
    pub(crate) fn ui_config_snapshot(&self) -> UiConfig {
        self.ui_config()
    }

    pub(crate) fn capture_device_catalog_status(&self) -> (usize, bool) {
        let snapshot = self.0.settings.device_catalog_snapshot(|| {
            self.publish_settings_effects(SettingsMutationEffects::SETTINGS_AND_MAIN);
        });
        (snapshot.devices.len(), snapshot.available)
    }

    pub(crate) fn set_onboarding_progress(
        &self,
        step: usize,
        done: bool,
    ) -> Result<bool, SettingsServiceError> {
        self.0.desktop.set_onboarding_step(step);
        self.bump_main_dps_revision();
        self.update_ui_config_with_effects(SettingsMutationEffects::NONE, |config| {
            config.onboarding_done = done;
        })
    }

    pub(crate) fn finish_onboarding(
        &self,
        preset: HudPreset,
    ) -> Result<bool, SettingsServiceError> {
        let changed = self.update_ui_config(|config| {
            let width = config.hud.width;
            let module_order = config.hud.module_order.clone();
            let mut hud = match preset {
                HudPreset::Minimal => HudConfig::minimal(),
                HudPreset::Standard => HudConfig::default(),
                HudPreset::Detailed => HudConfig::detailed(),
            };
            hud.width = width;
            hud.module_order = module_order;
            config.hud = hud;
            config.onboarding_done = true;
        })?;
        self.0.desktop.set_onboarding_step(3);
        self.bump_main_dps_revision();
        Ok(changed)
    }

    pub(crate) fn onboarding_step(&self) -> usize {
        self.0.desktop.onboarding_step()
    }

    pub(crate) fn console_window_geometry(&self) -> (Option<[f32; 2]>, Option<[f32; 2]>) {
        let config = self.ui_config();
        (config.console_window_size, config.console_window_position)
    }

    pub(crate) fn abyss_window_geometry(&self) -> (Option<[f32; 2]>, Option<[f32; 2]>) {
        let config = self.ui_config();
        (config.abyss_window_size, config.abyss_window_position)
    }

    pub(crate) fn set_abyss_window_geometry(
        &self,
        size: [f32; 2],
        position: [f32; 2],
    ) -> Result<bool, SettingsServiceError> {
        self.update_ui_config_with_effects(SettingsMutationEffects::NONE, |config| {
            config.abyss_window_size = Some(size);
            config.abyss_window_position = Some(position);
        })
    }

    pub(crate) fn set_console_window_geometry(
        &self,
        size: [f32; 2],
        position: [f32; 2],
    ) -> Result<bool, SettingsServiceError> {
        self.update_ui_config_with_effects(SettingsMutationEffects::NONE, |config| {
            config.console_window_size = Some(size);
            config.console_window_position = Some(position);
        })
    }

    pub(crate) fn set_hit_detail_columns(
        &self,
        columns: nte_dps_tool::storage::config::HitDetailColumnsConfig,
    ) -> Result<bool, SettingsServiceError> {
        self.update_ui_config_with_effects(SettingsMutationEffects::MAIN, |config| {
            config.hit_detail_columns = columns;
        })
    }

    pub(crate) fn main_dps_detail_window_geometry(
        &self,
        kind: MainDpsDetailKind,
    ) -> (Option<[f32; 2]>, Option<[f32; 2]>) {
        let config = self.ui_config();
        match kind {
            MainDpsDetailKind::Character => (
                config.hit_detail_window_size,
                config.hit_detail_window_position,
            ),
            MainDpsDetailKind::Team => (
                config.team_hit_detail_window_size,
                config.team_hit_detail_window_position,
            ),
        }
    }

    pub(crate) fn set_main_dps_detail_window_geometry(
        &self,
        size: [f32; 2],
        position: [f32; 2],
        kind: MainDpsDetailKind,
    ) -> Result<bool, SettingsServiceError> {
        self.update_ui_config_with_effects(SettingsMutationEffects::NONE, |config| match kind {
            MainDpsDetailKind::Character => {
                config.hit_detail_window_size = Some(size);
                config.hit_detail_window_position = Some(position);
            }
            MainDpsDetailKind::Team => {
                config.team_hit_detail_window_size = Some(size);
                config.team_hit_detail_window_position = Some(position);
            }
        })
    }

    pub(crate) fn settings_snapshot(&self) -> SettingsSnapshot {
        let (generation, config, devices, teams, updates) = loop {
            let before = self.settings_revision();
            let config = self.ui_config();
            let devices = self.0.settings.device_catalog_snapshot(|| {
                self.publish_settings_effects(SettingsMutationEffects::SETTINGS_AND_MAIN);
            });
            let teams = self.0.team_import.status(|| {
                self.publish_settings_effects(SettingsMutationEffects::SETTINGS);
            });
            // Use the same config generation for both the main Settings DTO and
            // its nested update projection.
            let updates = self.update_settings_snapshot_for_config(&config);
            let after = self.settings_revision();
            if before == after {
                break (after, config, devices, teams, updates);
            }
        };
        // Capture-log scanning is filesystem I/O and intentionally happens only
        // after every settings/team/device lock has been released.
        let capture_files = scan_capture_logs(&capture_log_dir());
        SettingsSnapshot::from_config(
            &config,
            generation,
            self.always_on_top(),
            devices.devices,
            devices.available,
            capture_files,
            teams.available,
            teams.upper_imported,
            teams.lower_imported,
            updates,
            self.combat_clock_health().unwrap_or_else(|error| {
                log::warn!("project combat-clock health for Settings failed: {error:?}");
                nte_dps_tool::engine::model::CombatClockRuntimeHealth::Unknown
            }),
        )
    }

    pub(crate) fn update_settings_snapshot(&self) -> UpdateSettingsSnapshot {
        let config = self.ui_config();
        self.update_settings_snapshot_for_config(&config)
    }

    fn update_settings_snapshot_for_config(&self, config: &UiConfig) -> UpdateSettingsSnapshot {
        let (update, install_blocked_message_key) = match self.0.update_runtime.snapshot() {
            Ok(update) => {
                let install_blocked_message_key = self.install_blocked_message_key_for(
                    update.prepared.as_ref().map(PreparedUpdate::component),
                );
                (update, install_blocked_message_key)
            }
            Err(UpdateActionError::RuntimeUnavailable) => {
                (UpdateRuntimeSnapshot::unavailable(), None)
            }
            Err(error) => {
                debug_assert!(false, "unexpected update snapshot error: {error:?}");
                (UpdateRuntimeSnapshot::unavailable(), None)
            }
        };
        UpdateSettingsSnapshot::from_runtime(
            config,
            update.status,
            update.message_key,
            update.message_arguments,
            &update.available,
            update.active_component,
            update.downloaded_bytes,
            update.total_bytes,
            update.prepared.as_ref(),
            install_blocked_message_key,
        )
    }

    pub(crate) fn passthrough(&self) -> bool {
        self.0.desktop.passthrough()
    }

    pub(crate) fn set_passthrough(&self, enabled: bool) {
        if self.0.desktop.set_passthrough(enabled) {
            self.0.presentation.revision.fetch_add(1, Ordering::AcqRel);
        }
    }

    pub(crate) fn passthrough_hotkey(&self) -> HotkeyBinding {
        self.ui_config().passthrough_hotkey
    }

    pub(crate) fn global_hotkeys(&self) -> GlobalHotkeys {
        self.ui_config().global_hotkeys
    }

    pub(crate) fn passthrough_hotkey_ready(&self) -> bool {
        self.0.desktop.passthrough_hotkey_ready()
    }

    pub(crate) fn set_passthrough_hotkey_ready(&self, ready: bool) {
        self.0.desktop.set_passthrough_hotkey_ready(ready);
    }

    pub(crate) fn lock_passthrough_transaction(
        &self,
    ) -> Result<PassthroughTransactionGuard<'_>, SettingsServiceError> {
        self.0.settings.lock_passthrough_transaction()
    }

    pub(crate) fn always_on_top(&self) -> bool {
        self.0.desktop.hud_always_on_top()
    }

    pub(crate) fn window_always_on_top(&self, window: DesktopWindowKind) -> bool {
        let config = self.ui_config();
        match window {
            DesktopWindowKind::MainDps => config.main_dps_always_on_top,
            DesktopWindowKind::Hud => config.hud_always_on_top,
            DesktopWindowKind::Console => config.console_always_on_top,
            DesktopWindowKind::AbyssValues => config.abyss_values_always_on_top,
            DesktopWindowKind::CharacterDetails => config.character_details_always_on_top,
            DesktopWindowKind::TeamDetails => config.team_details_always_on_top,
        }
        .expect("sanitized per-window always-on-top state")
    }

    pub(crate) fn hud_width(&self) -> u16 {
        self.hud_config().width
    }

    pub(crate) fn hud_window_position(&self) -> Option<[i32; 2]> {
        self.ui_config().hud_window_position
    }

    pub(crate) fn hud_initial_height(&self) -> u16 {
        let hud_config = self.hud_config();
        let content_height = HUD_BASE_INITIAL_HEIGHT
            + if hud_config.has_summary_row() {
                HUD_SUMMARY_HEIGHT
            } else {
                0
            }
            + if hud_config.show_character_rows {
                HUD_CHARACTERS_HEIGHT
            } else {
                0
            }
            + if hud_config.show_title {
                HUD_OPTIONAL_TITLE_HEIGHT
            } else {
                0
            }
            + if hud_config.show_abyss_half || hud_config.show_passthrough_state {
                HUD_OPTIONAL_STATUS_HEIGHT
            } else {
                0
            }
            + if hud_config.show_mini_timeline {
                HUD_MINI_TIMELINE_HEIGHT
            } else {
                0
            };
        if self.passthrough() {
            content_height
        } else {
            let editor_headers = hud_config
                .module_order
                .iter()
                .filter(|module| hud_config.module_visible(**module))
                .count() as u16
                * HUD_EDITOR_MODULE_HEADER_HEIGHT;
            (content_height + editor_headers).max(HUD_EDITOR_MIN_HEIGHT)
        }
    }

    pub(crate) fn set_hud_module_visibility(
        &self,
        module: HudModule,
        visible: bool,
    ) -> Result<bool, SettingsServiceError> {
        self.update_hud_config(|hud| hud.set_module_visible(module, visible))
    }

    pub(crate) fn move_hud_module(
        &self,
        dragged: HudModule,
        target: HudModule,
        insert_after: bool,
    ) -> Result<bool, SettingsServiceError> {
        self.update_hud_config(|hud| hud.move_module(dragged, target, insert_after))
    }

    pub(crate) fn set_hud_width(&self, width: u16) -> Result<bool, SettingsServiceError> {
        self.update_hud_config(|hud| hud.width = width)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn update_interface_settings(
        &self,
        language: Language,
        dark_mode: bool,
        theme_preset: ThemePreset,
        accent: AccentColor,
        density: UiDensity,
        reduce_motion: bool,
        island_notifications: bool,
        island_offset_x: f32,
    ) -> Result<bool, SettingsServiceError> {
        self.update_ui_config_with_effects_for_change(
            |change| SettingsMutationEffects {
                history: change.previous.language != change.current.language,
                ..SettingsMutationEffects::SETTINGS_AND_MAIN
            },
            |config| {
                config.language = language;
                config.dark_mode = dark_mode;
                config.theme_preset = theme_preset;
                config.accent = accent;
                config.density = density;
                config.reduce_motion = reduce_motion;
                config.island_notifications = island_notifications;
                config.island_offset_x = island_offset_x;
            },
        )
    }

    pub(crate) fn update_update_settings(
        &self,
        auto_check: bool,
        auto_download: bool,
    ) -> Result<bool, SettingsServiceError> {
        self.update_ui_config_with_effects(SettingsMutationEffects::SETTINGS, |config| {
            config.auto_check_updates = auto_check;
            config.auto_download_updates = auto_download;
        })
    }

    pub(crate) fn auto_check_updates(&self) -> bool {
        self.ui_config().auto_check_updates
    }

    pub(crate) fn auto_download_updates(&self) -> bool {
        self.ui_config().auto_download_updates
    }

    pub(crate) fn begin_update_check(&self) -> Result<(), UpdateActionError> {
        self.0.update_runtime.begin_check()?;
        self.bump_settings_revision();
        Ok(())
    }

    pub(crate) fn finish_update_check(
        &self,
        available: Vec<AvailableComponentUpdate>,
    ) -> Result<(), UpdateActionError> {
        self.0.update_runtime.finish_check(available)?;
        self.bump_settings_revision();
        Ok(())
    }

    pub(crate) fn fail_update_check(
        &self,
        message_key: &'static str,
    ) -> Result<(), UpdateActionError> {
        self.0.update_runtime.fail_check(message_key)?;
        self.bump_settings_revision();
        Ok(())
    }

    pub(crate) fn begin_update_download(
        &self,
        component: UpdateComponent,
    ) -> Result<AvailableComponentUpdate, UpdateActionError> {
        let selected = self.0.update_runtime.begin_download(component)?;
        self.bump_settings_revision();
        Ok(selected)
    }

    pub(crate) fn update_download_progress(
        &self,
        component: UpdateComponent,
        downloaded_bytes: u64,
        total_bytes: u64,
    ) -> Result<bool, UpdateActionError> {
        let changed = self.0.update_runtime.update_download_progress(
            component,
            downloaded_bytes,
            total_bytes,
        )?;
        if changed {
            self.bump_settings_revision();
        }
        Ok(changed)
    }

    pub(crate) fn finish_update_download(
        &self,
        prepared: PreparedUpdate,
    ) -> Result<(), UpdateActionError> {
        self.0.update_runtime.finish_download(prepared)?;
        self.bump_settings_revision();
        Ok(())
    }

    pub(crate) fn fail_update_download(&self) -> Result<(), UpdateActionError> {
        self.fail_update_operation("Update download failed.")
    }

    pub(crate) fn begin_update_install(&self) -> Result<PreparedUpdate, UpdateActionError> {
        let prepared = self.0.update_runtime.begin_install()?;
        self.bump_settings_revision();
        Ok(prepared)
    }

    pub(crate) fn finish_plugin_update_install(
        &self,
        version: String,
    ) -> Result<(), UpdateActionError> {
        self.0.update_runtime.finish_plugin_install(version)?;
        self.bump_settings_revision();
        Ok(())
    }

    pub(crate) fn fail_update_install(&self) -> Result<(), UpdateActionError> {
        self.fail_update_operation("Update installation could not start.")
    }

    pub(crate) fn update_install_blocked_message_key(
        &self,
    ) -> Result<Option<&'static str>, UpdateActionError> {
        let prepared_component = self.0.update_runtime.prepared_component()?;
        Ok(self.install_blocked_message_key_for(prepared_component))
    }

    pub(crate) fn notify_update_install_blocker_changed(&self) {
        self.bump_settings_revision();
    }

    fn install_blocked_message_key_for(
        &self,
        prepared_component: Option<UpdateComponent>,
    ) -> Option<&'static str> {
        match prepared_component {
            Some(UpdateComponent::App)
                if !matches!(
                    self.capture_phase(),
                    LiveCapturePhase::Idle | LiveCapturePhase::Stopped | LiveCapturePhase::Failed
                ) =>
            {
                Some("Stop capture or replay before installing the update")
            }
            Some(UpdateComponent::ModsPlugin) => {
                match nte_dps_tool::platform::network::game_process_is_running() {
                    Ok(true) => Some("Close HTGame.exe before installing the Mod loader update"),
                    Ok(false) => None,
                    Err(error) => {
                        log::error!("check game process before Mod loader update failed: {error}");
                        Some("Game process state could not be checked.")
                    }
                }
            }
            _ => None,
        }
    }

    fn fail_update_operation(&self, message_key: &'static str) -> Result<(), UpdateActionError> {
        self.0.update_runtime.fail_operation(message_key)?;
        self.bump_settings_revision();
        Ok(())
    }

    fn bump_settings_revision(&self) {
        self.0.desktop.bump_settings_revision();
    }

    pub(crate) fn set_density(&self, density: UiDensity) -> Result<bool, SettingsServiceError> {
        self.update_ui_config_with_effects(SettingsMutationEffects::SETTINGS_AND_MAIN, |config| {
            config.density = density
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn update_capture_settings(
        &self,
        filter: String,
        manual_capture_device: Option<String>,
        server_damage_calibration: bool,
        include_max_hp_reduction_in_total_damage: bool,
        separate_reaction_damage: bool,
        auto_round_after_idle: bool,
        auto_round_idle_seconds: u32,
        passthrough_hotkey: HotkeyBinding,
    ) -> Result<bool, SettingsServiceError> {
        let changed = self.update_ui_config(|config| {
            config.capture_filter = filter;
            config.manual_capture_device = manual_capture_device;
            config.server_damage_calibration = server_damage_calibration;
            config.include_max_hp_reduction_in_total_damage =
                include_max_hp_reduction_in_total_damage;
            config.separate_reaction_damage = separate_reaction_damage;
            config.auto_round_after_idle = auto_round_after_idle;
            config.auto_round_idle_seconds = auto_round_idle_seconds;
            config.passthrough_hotkey = passthrough_hotkey;
        })?;
        // Keep the capture-side automatic round-boundary policy synchronized
        // only after the durable settings transaction succeeds. A boundary
        // then freezes this effective basis and reaction mode together with
        // its detached state instead of consulting mutable settings later.
        self.0
            .live_capture
            .set_history_archive_policy(history_archive_policy(&self.ui_config()));
        Ok(changed)
    }

    pub(crate) fn update_global_hotkeys(
        &self,
        global_hotkeys: GlobalHotkeys,
    ) -> Result<bool, SettingsServiceError> {
        self.update_ui_config_with_effects(SettingsMutationEffects::SETTINGS, |config| {
            config.global_hotkeys = global_hotkeys;
        })
    }

    pub(crate) fn update_main_dps_display(
        &self,
        display: MainDpsDisplayConfig,
    ) -> Result<bool, SettingsServiceError> {
        self.update_ui_config_with_effects(SettingsMutationEffects::SETTINGS_AND_MAIN, |config| {
            config.main_dps_display = display
        })
    }

    pub(crate) fn refresh_capture_devices(&self) -> Result<(), CoreError> {
        self.apply_capture_device_refresh(self.0.settings.refresh_devices(|| {
            self.publish_settings_effects(SettingsMutationEffects::SETTINGS_AND_MAIN);
        }))
    }

    pub(super) fn apply_capture_device_refresh(
        &self,
        outcome: crate::settings_service::DeviceRefreshOutcome,
    ) -> Result<(), CoreError> {
        match outcome.error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    pub(crate) fn clear_capture_files(&self) -> ClearOutcome {
        clear_capture_logs(&capture_log_dir())
    }

    pub(crate) fn refresh_capture_file_stats(&self) {
        self.bump_settings_revision();
    }

    pub(crate) fn set_hud_option(
        &self,
        option: HudSettingOption,
        enabled: bool,
    ) -> Result<bool, SettingsServiceError> {
        self.update_hud_config(|hud| match option {
            HudSettingOption::Title => hud.show_title = enabled,
            HudSettingOption::TeamDps => hud.show_team_dps = enabled,
            HudSettingOption::Duration => hud.show_duration = enabled,
            HudSettingOption::TotalDamage => hud.show_total_damage = enabled,
            HudSettingOption::DamageTaken => hud.show_damage_taken = enabled,
            HudSettingOption::CharacterRows => hud.show_character_rows = enabled,
            HudSettingOption::AbyssHalf => hud.show_abyss_half = enabled,
            HudSettingOption::PassthroughState => hud.show_passthrough_state = enabled,
            HudSettingOption::MiniTimeline => hud.show_mini_timeline = enabled,
        })
    }

    pub(crate) fn apply_hud_preset(&self, preset: HudPreset) -> Result<bool, SettingsServiceError> {
        self.update_hud_config(|hud| {
            let width = hud.width;
            let module_order = hud.module_order.clone();
            let mut candidate = match preset {
                HudPreset::Minimal => HudConfig::minimal(),
                HudPreset::Standard => HudConfig::default(),
                HudPreset::Detailed => HudConfig::detailed(),
            };
            candidate.width = width;
            candidate.module_order = module_order;
            *hud = candidate;
        })
    }

    pub(crate) fn set_hud_window_position(
        &self,
        position: [i32; 2],
    ) -> Result<bool, SettingsServiceError> {
        self.update_ui_config_with_effects(SettingsMutationEffects::NONE, |config| {
            config.hud_window_position = Some(position);
        })
    }

    fn update_hud_config(
        &self,
        update: impl FnOnce(&mut HudConfig),
    ) -> Result<bool, SettingsServiceError> {
        self.update_ui_config(|config| update(&mut config.hud))
    }

    pub(crate) fn set_always_on_top(&self, enabled: bool) -> Result<bool, SettingsServiceError> {
        self.set_window_always_on_top(DesktopWindowKind::Hud, enabled)
    }

    pub(crate) fn set_window_always_on_top(
        &self,
        window: DesktopWindowKind,
        enabled: bool,
    ) -> Result<bool, SettingsServiceError> {
        let effects = match window {
            DesktopWindowKind::Hud => SettingsMutationEffects::SETTINGS_AND_TECHNICAL,
            DesktopWindowKind::MainDps => SettingsMutationEffects::MAIN,
            DesktopWindowKind::Console
            | DesktopWindowKind::AbyssValues
            | DesktopWindowKind::CharacterDetails
            | DesktopWindowKind::TeamDetails => SettingsMutationEffects::NONE,
        };
        let changed = self.update_ui_config_with_effects(effects, |config| match window {
            DesktopWindowKind::MainDps => config.main_dps_always_on_top = Some(enabled),
            DesktopWindowKind::Hud => {
                config.always_on_top = enabled;
                config.hud_always_on_top = Some(enabled);
            }
            DesktopWindowKind::Console => config.console_always_on_top = Some(enabled),
            DesktopWindowKind::AbyssValues => config.abyss_values_always_on_top = Some(enabled),
            DesktopWindowKind::CharacterDetails => {
                config.character_details_always_on_top = Some(enabled)
            }
            DesktopWindowKind::TeamDetails => config.team_details_always_on_top = Some(enabled),
        })?;
        if window == DesktopWindowKind::Hud {
            self.0.desktop.set_hud_always_on_top(enabled);
        }
        Ok(changed)
    }

    pub(super) fn ui_config(&self) -> UiConfig {
        self.0.settings.config_snapshot()
    }

    pub(super) fn update_ui_config(
        &self,
        update: impl FnOnce(&mut UiConfig),
    ) -> Result<bool, SettingsServiceError> {
        self.update_ui_config_with_effects(SettingsMutationEffects::SETTINGS_AND_TECHNICAL, update)
    }

    pub(super) fn update_ui_config_with_effects(
        &self,
        effects: SettingsMutationEffects,
        update: impl FnOnce(&mut UiConfig),
    ) -> Result<bool, SettingsServiceError> {
        self.update_ui_config_with_effects_for_change(|_| effects, update)
    }

    fn update_ui_config_with_effects_for_change(
        &self,
        effects: impl FnOnce(&ConfigUpdate) -> SettingsMutationEffects,
        update: impl FnOnce(&mut UiConfig),
    ) -> Result<bool, SettingsServiceError> {
        let Some(update) = self.0.settings.update_config(update, |change| {
            self.publish_settings_effects(effects(change));
        })?
        else {
            return Ok(false);
        };
        debug_assert_ne!(update.previous, update.current);
        Ok(true)
    }

    pub(super) fn publish_settings_effects(&self, effects: SettingsMutationEffects) {
        if effects.settings {
            self.bump_settings_revision();
        }
        if effects.technical {
            self.0.presentation.revision.fetch_add(1, Ordering::AcqRel);
        }
        if effects.main {
            self.bump_main_dps_revision();
        }
        if effects.history {
            self.bump_history_revision();
        }
    }

    pub(crate) fn proxy_game_directory(&self) -> Option<String> {
        self.ui_config().mod_studio_proxy_game_directory
    }

    pub(crate) fn set_proxy_game_directory(
        &self,
        directory: Option<String>,
    ) -> Result<bool, SettingsServiceError> {
        self.update_ui_config_with_effects(SettingsMutationEffects::NONE, |config| {
            config.mod_studio_proxy_game_directory = directory;
        })
    }

    pub(crate) fn mod_studio_loading_method(&self) -> ModStudioLoadingMethod {
        self.ui_config().mod_studio_loading_method
    }

    pub(crate) fn set_mod_studio_loading_method(
        &self,
        method: ModStudioLoadingMethod,
    ) -> Result<bool, SettingsServiceError> {
        self.update_ui_config_with_effects(SettingsMutationEffects::NONE, |config| {
            config.mod_studio_loading_method = method;
        })
    }

    fn hud_config(&self) -> HudConfig {
        self.ui_config().hud
    }
}
