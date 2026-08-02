mod channels;
mod commands;
mod contract;
mod history_runtime;
mod state;
mod windows;

use state::AppState;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let (ui_config, config_warning) = nte_dps_tool::storage::config::load();
    nte_dps_tool::storage::i18n::set_language(ui_config.language);
    let (capture_resources, resource_warnings) =
        nte_dps_tool::core::live_capture::LiveCaptureResources::load(ui_config.language);
    if config_warning.is_some() || !resource_warnings.is_empty() {
        eprintln!("Tauri loaded startup resources with a recoverable warning");
    }
    let live_capture = nte_dps_tool::core::live_capture::LiveCaptureService::new(capture_resources);

    tauri::Builder::default()
        .manage(AppState::new(ui_config, live_capture))
        .setup(|app| {
            let state = app.state::<AppState>();
            let main_dps_window = app
                .get_webview_window(windows::main_dps::MAIN_DPS_WINDOW_LABEL)
                .expect("configured main DPS window must exist");
            let hud_window = app
                .get_webview_window(windows::hud::HUD_WINDOW_LABEL)
                .expect("configured HUD window must exist");
            let console_window = app
                .get_webview_window(windows::console::CONSOLE_WINDOW_LABEL)
                .expect("configured Console window must exist");
            let abyss_values_window = app
                .get_webview_window(windows::abyss_values::ABYSS_VALUES_WINDOW_LABEL)
                .expect("configured Abyss Values window must exist");
            let combat_details_window = app
                .get_webview_window(windows::combat_details::COMBAT_DETAILS_WINDOW_LABEL)
                .expect("configured combat details window must exist");
            windows::main_dps::initialize(
                &main_dps_window,
                app.handle().clone(),
                state.inner().clone(),
            );
            windows::console::bind_close_to_hide(&console_window);
            windows::abyss_values::bind_close_to_hide(&abyss_values_window);
            windows::combat_details::bind_close_to_hide(&combat_details_window);
            console_window.set_title(&nte_dps_tool::storage::i18n::t("NTE Console"))?;
            main_dps_window.set_always_on_top(state.always_on_top())?;
            hud_window.set_always_on_top(state.always_on_top())?;
            if let Err(error) = windows::hud::set_editing_effect(&hud_window, true) {
                eprintln!("Tauri HUD native editing effect unavailable: {error}");
            }
            if let Err(error) = windows::hud::set_native_shape(&hud_window) {
                eprintln!("Tauri HUD native rounded shape unavailable: {error}");
            }
            windows::hud::initialize_content_size(&hud_window, &state)?;
            if let Err(error) = windows::hud::restore_content_position(&hud_window, &state) {
                eprintln!("Tauri HUD position restore unavailable: {error}");
            }
            if let Err(error) = windows::hud::track_native_width(&hud_window, &state) {
                eprintln!("Tauri HUD native width persistence unavailable: {error}");
            }
            if let Err(error) = windows::hud::track_native_position(&hud_window, &state) {
                eprintln!("Tauri HUD native position persistence unavailable: {error}");
            }
            #[cfg(windows)]
            match windows::passthrough_hotkey::HudPassthroughHotkeyRuntime::start(
                app.handle().clone(),
                state.inner().clone(),
            ) {
                Ok(runtime) => {
                    let managed = app.manage(runtime);
                    debug_assert!(managed, "HUD hotkey runtime is managed once");
                }
                Err(error) => {
                    eprintln!("Tauri HUD passthrough hotkey unavailable: {error}");
                }
            }
            commands::settings::schedule_completed_update_cleanup();
            commands::settings::schedule_automatic_update_check(state.inner().clone());
            match history_runtime::HistoryRuntime::start(state.inner().clone()) {
                Ok(runtime) => {
                    let managed = app.manage(runtime);
                    debug_assert!(managed, "History runtime is managed once");
                }
                Err(error) => eprintln!("Tauri History maintenance unavailable: {error}"),
            }

            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::abyss_values::clear_abyss_prediction_team,
            commands::abyss_values::get_abyss_values_snapshot,
            commands::abyss_values::import_abyss_prediction_team,
            commands::abyss_values::swap_abyss_prediction_teams,
            commands::character_data::get_character_data_snapshot,
            commands::character_data::save_character_data_record,
            commands::diagnostics::export_diagnostics_json,
            commands::diagnostics::export_diagnostics_pcapng,
            commands::diagnostics::get_diagnostics_snapshot,
            commands::diagnostics::import_diagnostics_json,
            commands::diagnostics::import_diagnostics_pcapng,
            commands::diagnostics::run_diagnostics,
            commands::encrypted_ini::clear_encrypted_ini,
            commands::encrypted_ini::get_encrypted_ini_snapshot,
            commands::encrypted_ini::open_encrypted_ini,
            commands::encrypted_ini::reload_encrypted_ini,
            commands::encrypted_ini::save_encrypted_ini,
            commands::empty_curtain::apply_empty_curtain_character_action,
            commands::empty_curtain::export_empty_curtain_inventory,
            commands::empty_curtain::export_empty_curtain_loadout,
            commands::empty_curtain::get_empty_curtain_positions,
            commands::empty_curtain::get_empty_curtain_snapshot,
            commands::empty_curtain::import_empty_curtain_loadout,
            commands::empty_curtain::manage_empty_curtain_item,
            commands::history::compare_history_records,
            commands::history::delete_history_record,
            commands::history::export_history_record_json,
            commands::history::get_history_snapshot,
            commands::history::import_history_record_json,
            commands::history::restore_deleted_history_record,
            commands::history::save_current_history_summary,
            commands::history::set_history_prediction_team,
            commands::main_dps::close_main_dps_window,
            commands::main_dps::get_main_dps_snapshot,
            commands::main_dps::get_main_dps_detail_snapshot,
            commands::main_dps::import_main_dps_replay,
            commands::main_dps::minimize_main_dps_window,
            commands::main_dps::open_main_dps_console,
            commands::main_dps::open_main_dps_character_details,
            commands::main_dps::open_main_dps_team_details,
            commands::main_dps::open_main_dps_hud,
            commands::main_dps::reset_main_dps_session,
            commands::main_dps::select_main_dps_abyss_half,
            commands::main_dps::select_main_dps_round,
            commands::main_dps::set_main_dps_always_on_top,
            commands::main_dps::set_main_dps_appearance,
            commands::main_dps::set_main_dps_paused,
            commands::main_dps::set_main_dps_passthrough,
            commands::main_dps::show_main_dps_from_hud,
            commands::main_dps::start_main_dps_capture,
            commands::main_dps::start_main_dps_new_round,
            commands::main_dps::stop_main_dps_capture,
            commands::main_dps::toggle_main_dps_maximized,
            commands::mod_studio::get_mod_studio_document,
            commands::mod_studio::get_mod_studio_sdk_schema,
            commands::mod_studio::get_mod_studio_workspace,
            commands::mod_studio::save_mod_studio_document,
            commands::mod_studio::set_mod_studio_document_enabled,
            commands::packets::get_packets_snapshot,
            commands::resources::get_resources_snapshot,
            commands::settings::apply_settings_hud_preset,
            commands::settings::apply_settings_layout_profile,
            commands::settings::clear_settings_capture_files,
            commands::settings::check_settings_updates,
            commands::settings::download_settings_update,
            commands::settings::export_settings_team_data,
            commands::settings::get_settings_snapshot,
            commands::settings::import_settings_team_data,
            commands::settings::install_settings_update,
            commands::settings::move_settings_hud_module,
            commands::settings::open_settings_hud_editor,
            commands::settings::open_settings_abyss_values,
            commands::settings::refresh_settings_capture_devices,
            commands::settings::refresh_settings_capture_files,
            commands::settings::set_settings_capture,
            commands::settings::set_settings_hud_always_on_top,
            commands::settings::set_settings_hud_module_visibility,
            commands::settings::set_settings_hud_option,
            commands::settings::set_settings_hud_width,
            commands::settings::set_settings_hotkey_binding,
            commands::settings::set_settings_hotkeys_enabled,
            commands::settings::set_settings_interface,
            commands::settings::set_settings_update_preferences,
            commands::skills::get_skills_snapshot,
            commands::timeline::get_timeline_snapshot,
            commands::timeline::set_timeline_preferences,
            commands::technical::get_technical_snapshot,
            commands::technical::move_hud_module,
            commands::technical::set_hud_always_on_top,
            commands::technical::set_hud_module_visibility,
            commands::technical::set_hud_passthrough,
            commands::technical::set_hud_width,
            commands::technical::start_hud_capture,
            commands::technical::stop_hud_capture,
            windows::console::show_console_when_ready,
            windows::main_dps::show_main_dps_when_ready,
            channels::technical::subscribe_technical_state,
            channels::technical::unsubscribe_technical_state,
            channels::diagnostics::subscribe_diagnostics,
            channels::diagnostics::unsubscribe_diagnostics,
            channels::history::subscribe_history,
            channels::history::unsubscribe_history,
            channels::main_dps::subscribe_main_dps,
            channels::main_dps::unsubscribe_main_dps,
            channels::empty_curtain::subscribe_empty_curtain,
            channels::empty_curtain::unsubscribe_empty_curtain,
            channels::mod_studio::subscribe_mod_studio_runtime,
            channels::mod_studio::unsubscribe_mod_studio_runtime,
            channels::packets::subscribe_packets,
            channels::packets::unsubscribe_packets,
            channels::settings::subscribe_settings,
            channels::settings::unsubscribe_settings,
            channels::skills::subscribe_skills,
            channels::skills::unsubscribe_skills,
            channels::timeline::subscribe_timeline,
            channels::timeline::unsubscribe_timeline,
        ])
        .run(tauri::generate_context!())
        .expect("Tauri application runtime failed");
}
