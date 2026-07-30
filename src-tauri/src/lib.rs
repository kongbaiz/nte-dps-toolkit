mod channels;
mod commands;
mod contract;
mod state;
mod windows;

use state::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(AppState::default())
        .setup(|app| {
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
            commands::technical::get_technical_snapshot,
            commands::technical::set_hud_always_on_top,
            commands::technical::set_hud_passthrough,
            channels::technical::subscribe_technical_state,
            channels::technical::unsubscribe_technical_state,
        ])
        .run(tauri::generate_context!())
        .expect("Tauri application runtime failed");
}
