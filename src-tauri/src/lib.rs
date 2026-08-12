mod channels;
mod commands;
mod contract;
mod history_runtime;
mod state;
mod windows;

use state::{AppState, DesktopWindowKind};
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
        .on_page_load(|webview, payload| {
            if windows::main_dps::should_reveal_after_page_load(webview.label(), payload.event()) {
                let window = webview.window();
                let reveal = window
                    .show()
                    .and_then(|()| window.unminimize())
                    .and_then(|()| window.set_focus());
                if let Err(error) = reveal {
                    eprintln!("Tauri main DPS native page-load reveal failed: {error}");
                }
            }
        })
        .on_window_event(|window, event| {
            if matches!(event, tauri::WindowEvent::Destroyed) {
                window
                    .state::<AppState>()
                    .stop_streams_for_window(window.label());
            }
        })
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
            let character_details_window = app
                .get_webview_window(windows::combat_details::CHARACTER_DETAILS_WINDOW_LABEL)
                .expect("configured character details window must exist");
            let team_details_window = app
                .get_webview_window(windows::combat_details::TEAM_DETAILS_WINDOW_LABEL)
                .expect("configured team details window must exist");
            let island_window = app
                .get_webview_window(windows::island::ISLAND_WINDOW_LABEL)
                .expect("configured notification island window must exist");
            windows::main_dps::initialize(
                &main_dps_window,
                app.handle().clone(),
                state.inner().clone(),
            );
            windows::console::initialize(&console_window, state.inner())?;
            windows::abyss_values::initialize(&abyss_values_window, state.inner())?;
            windows::combat_details::initialize(
                &character_details_window,
                state.inner(),
                state::MainDpsDetailKind::Character,
            )?;
            windows::combat_details::initialize(
                &team_details_window,
                state.inner(),
                state::MainDpsDetailKind::Team,
            )?;
            windows::island::bind_close_to_hide(&island_window);
            if let Err(error) = island_window.set_focusable(false) {
                log::warn!("disable notification island focus failed: {error}");
            }
            console_window.set_title(&nte_dps_tool::storage::i18n::t("NTE Console"))?;
            for (window, kind) in [
                (&main_dps_window, DesktopWindowKind::MainDps),
                (&hud_window, DesktopWindowKind::Hud),
                (&console_window, DesktopWindowKind::Console),
                (&abyss_values_window, DesktopWindowKind::AbyssValues),
                (
                    &character_details_window,
                    DesktopWindowKind::CharacterDetails,
                ),
                (&team_details_window, DesktopWindowKind::TeamDetails),
            ] {
                window.set_always_on_top(state.window_always_on_top(kind))?;
            }
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
            commands::settings::schedule_automatic_update_check(
                app.handle().clone(),
                state.inner().clone(),
            );
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
            commands::abyss_values::import_current_abyss_prediction_team,
            commands::abyss_values::swap_abyss_prediction_teams,
            commands::character_data::get_character_data_snapshot,
            commands::character_data::save_character_data_record,
            commands::console_control::execute_console_control,
            commands::diagnostics::export_diagnostics_json,
            commands::diagnostics::export_diagnostics_pcapng,
            commands::diagnostics::get_diagnostics_snapshot,
            commands::diagnostics::import_diagnostics_json,
            commands::diagnostics::import_diagnostics_pcapng,
            commands::diagnostics::run_diagnostics,
            commands::desktop_window::set_desktop_window_always_on_top,
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
            commands::history::export_history_record_file,
            commands::history::get_history_snapshot,
            commands::history::import_history_record_file,
            commands::history::import_history_record_json,
            commands::history::restore_deleted_history_record,
            commands::history::save_current_history_summary,
            commands::history::set_history_prediction_team,
            commands::island::dismiss_island_notice,
            commands::island::get_island_snapshot,
            commands::island::undo_island_notice,
            commands::main_dps::close_main_dps_window,
            commands::main_dps::get_main_dps_snapshot,
            commands::main_dps::get_main_dps_update_prompt,
            commands::main_dps::get_main_dps_detail_snapshot,
            commands::main_dps::download_main_dps_update,
            commands::main_dps::finish_main_dps_onboarding,
            commands::main_dps::set_main_dps_detail_view,
            commands::main_dps::set_main_dps_detail_columns,
            commands::main_dps::import_main_dps_replay,
            commands::main_dps::import_main_dps_replay_path,
            commands::main_dps::install_main_dps_update,
            commands::main_dps::minimize_main_dps_window,
            commands::main_dps::open_main_dps_console,
            commands::main_dps::open_main_dps_character_details,
            commands::main_dps::open_main_dps_team_details,
            commands::main_dps::open_main_dps_hud,
            commands::main_dps::open_main_dps_console_shortcut,
            commands::main_dps::reset_main_dps_session,
            commands::main_dps::select_main_dps_abyss_half,
            commands::main_dps::select_main_dps_round,
            commands::main_dps::set_main_dps_always_on_top,
            commands::main_dps::set_main_dps_appearance,
            commands::main_dps::set_main_dps_onboarding_step,
            commands::main_dps::set_main_dps_paused,
            commands::main_dps::set_main_dps_passthrough,
            commands::main_dps::show_main_dps_from_hud,
            commands::main_dps::start_main_dps_capture,
            commands::main_dps::start_main_dps_new_round,
            commands::main_dps::stop_main_dps_capture,
            commands::main_dps::toggle_main_dps_maximized,
            commands::main_dps::undo_main_dps_reset,
            commands::mod_studio::get_mod_studio_document,
            commands::mod_studio::get_mod_market_catalog,
            commands::mod_studio::install_mod_market_item,
            commands::mod_studio::get_mod_studio_sdk_schema,
            commands::mod_studio::get_mod_studio_workspace,
            commands::mod_studio::create_mod_studio_document,
            commands::mod_studio::delete_mod_studio_document,
            commands::mod_studio::open_mod_studio_folder,
            commands::mod_studio::get_mod_studio_deployment,
            commands::mod_studio::get_mod_studio_game_directory,
            commands::mod_studio::choose_mod_studio_game_directory,
            commands::mod_studio::set_mod_studio_game_directory,
            commands::mod_studio::set_mod_studio_loader_enabled,
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
            commands::settings::import_settings_team_data_file,
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
            channels::main_dps_detail::subscribe_main_dps_detail,
            channels::main_dps_detail::unsubscribe_main_dps_detail,
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
