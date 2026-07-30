use tauri::{State, WebviewWindow};

use nte_dps_tool::storage::config::{HUD_WIDTH_MAX, HUD_WIDTH_MIN, HudModule};

use crate::{
    contract::{CommandError, TechnicalSnapshot},
    state::AppState,
    windows::hud,
};

#[tauri::command]
pub(crate) fn get_technical_snapshot(
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<TechnicalSnapshot, CommandError> {
    hud::validate_window(&window)?;
    Ok(state.snapshot())
}

#[tauri::command]
pub(crate) fn set_hud_passthrough(
    enabled: bool,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<TechnicalSnapshot, CommandError> {
    hud::validate_window(&window)?;
    hud::set_passthrough(&window, &state, enabled)?;
    Ok(state.snapshot())
}

#[tauri::command]
pub(crate) fn set_hud_always_on_top(
    enabled: bool,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<TechnicalSnapshot, CommandError> {
    hud::validate_window(&window)?;
    hud::set_always_on_top(&window, &state, enabled)?;
    Ok(state.snapshot())
}

#[tauri::command]
pub(crate) fn move_hud_module(
    dragged: String,
    target: String,
    insert_after: bool,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<TechnicalSnapshot, CommandError> {
    hud::validate_window(&window)?;
    let dragged = parse_hud_module(&dragged)?;
    let target = parse_hud_module(&target)?;
    state
        .move_hud_module(dragged, target, insert_after)
        .map_err(|error| {
            log::error!("save HUD module order failed: {error}");
            CommandError::hud_config_save_failed()
        })?;
    Ok(state.snapshot())
}

#[tauri::command]
pub(crate) fn set_hud_module_visibility(
    module: String,
    visible: bool,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<TechnicalSnapshot, CommandError> {
    hud::validate_window(&window)?;
    let module = parse_hud_module(&module)?;
    let changed = state
        .set_hud_module_visibility(module, visible)
        .map_err(|error| {
            log::error!("save HUD module visibility failed: {error}");
            CommandError::hud_config_save_failed()
        })?;
    if changed && hud::sync_content_height(&window, &state).is_err() {
        log::warn!("native HUD height refresh failed after module visibility change");
    }
    Ok(state.snapshot())
}

#[tauri::command]
pub(crate) fn set_hud_width(
    width: i32,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<TechnicalSnapshot, CommandError> {
    hud::validate_window(&window)?;
    let width = sanitize_hud_width(width);
    let changed = state.set_hud_width(width).map_err(|error| {
        log::error!("save HUD width failed: {error}");
        CommandError::hud_config_save_failed()
    })?;
    if changed && hud::sync_content_width(&window, &state).is_err() {
        log::warn!("native HUD width refresh failed after configuration change");
    }
    Ok(state.snapshot())
}

#[tauri::command]
pub(crate) fn start_hud_capture(
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<TechnicalSnapshot, CommandError> {
    hud::validate_window(&window)?;
    state
        .request_capture_start()
        .map_err(CommandError::from_core)?;
    Ok(state.snapshot())
}

#[tauri::command]
pub(crate) fn stop_hud_capture(
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<TechnicalSnapshot, CommandError> {
    hud::validate_window(&window)?;
    state
        .request_capture_stop()
        .map_err(CommandError::from_core)?;
    Ok(state.snapshot())
}

fn parse_hud_module(module: &str) -> Result<HudModule, CommandError> {
    match module {
        "title" => Ok(HudModule::Title),
        "summary" => Ok(HudModule::Summary),
        "status" => Ok(HudModule::Status),
        "characters" => Ok(HudModule::Characters),
        "timeline" => Ok(HudModule::Timeline),
        _ => Err(CommandError::invalid_hud_module()),
    }
}

fn sanitize_hud_width(width: i32) -> u16 {
    width.clamp(i32::from(HUD_WIDTH_MIN), i32::from(HUD_WIDTH_MAX)) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hud_module_input_accepts_only_the_stable_contract_values() {
        assert_eq!(parse_hud_module("title").expect("title"), HudModule::Title);
        assert_eq!(
            parse_hud_module("characters").expect("characters"),
            HudModule::Characters
        );
        assert_eq!(
            parse_hud_module("timeline").expect("timeline"),
            HudModule::Timeline
        );
        assert_eq!(
            parse_hud_module("status").expect("status"),
            HudModule::Status
        );
        assert!(parse_hud_module("future_module").is_err());
        assert!(parse_hud_module("../summary").is_err());
    }

    #[test]
    fn hud_width_input_uses_the_existing_config_bounds() {
        assert_eq!(sanitize_hud_width(i32::MIN), HUD_WIDTH_MIN);
        assert_eq!(sanitize_hud_width(512), 512);
        assert_eq!(sanitize_hud_width(i32::MAX), HUD_WIDTH_MAX);
    }
}
