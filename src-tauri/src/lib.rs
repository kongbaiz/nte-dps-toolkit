mod channels;
mod commands;
mod contract;
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
            let hud_window = app
                .get_webview_window(windows::hud::HUD_WINDOW_LABEL)
                .expect("configured HUD window must exist");
            let console_window = app
                .get_webview_window(windows::console::CONSOLE_WINDOW_LABEL)
                .expect("configured Console window must exist");
            console_window.set_title(&nte_dps_tool::storage::i18n::t("NTE Console"))?;
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
            commands::mod_studio::get_mod_studio_document,
            commands::mod_studio::get_mod_studio_workspace,
            commands::technical::get_technical_snapshot,
            commands::technical::move_hud_module,
            commands::technical::set_hud_always_on_top,
            commands::technical::set_hud_module_visibility,
            commands::technical::set_hud_passthrough,
            commands::technical::set_hud_width,
            commands::technical::start_hud_capture,
            commands::technical::stop_hud_capture,
            channels::technical::subscribe_technical_state,
            channels::technical::unsubscribe_technical_state,
        ])
        .run(tauri::generate_context!())
        .expect("Tauri application runtime failed");
}
