use tauri::{
    AppHandle, LogicalPosition, LogicalSize, Manager, Position, Size, WebviewWindow, WindowEvent,
    webview::PageLoadEvent,
};

use crate::{
    contract::CommandError,
    state::AppState,
    windows::{
        abyss_values::ABYSS_VALUES_WINDOW_LABEL, combat_details::COMBAT_DETAILS_WINDOW_LABEL,
        console::CONSOLE_WINDOW_LABEL, hud::HUD_WINDOW_LABEL,
    },
};

pub(crate) const MAIN_DPS_WINDOW_LABEL: &str = "main-dps";
const LEGACY_TAURI_MAIN_DPS_DEFAULT_SIZE: [f32; 2] = [670.0, 692.0];
const COMPACT_TAURI_MAIN_DPS_DEFAULT_SIZE: [f32; 2] = [670.0, 560.0];
// Windows moves minimized top-level windows to a large negative sentinel position.
// The stored value is expressed in logical points, so the exact sentinel varies
// with DPI; both axes remain far outside any practical monitor arrangement.
const MINIMIZED_POSITION_LIMIT: f32 = -10_000.0;

pub(crate) fn should_reveal_after_page_load(label: &str, event: PageLoadEvent) -> bool {
    label == MAIN_DPS_WINDOW_LABEL && matches!(event, PageLoadEvent::Finished)
}

pub(crate) fn validate_window(window: &WebviewWindow) -> Result<(), CommandError> {
    if window.label() == MAIN_DPS_WINDOW_LABEL {
        Ok(())
    } else {
        Err(CommandError::invalid_window())
    }
}

pub(crate) fn set_passthrough(
    window: &WebviewWindow,
    state: &AppState,
    enabled: bool,
) -> Result<(), CommandError> {
    if enabled && !state.passthrough_hotkey_ready() {
        return Err(CommandError::passthrough_hotkey_unavailable());
    }
    window.set_ignore_cursor_events(enabled).map_err(|error| {
        log::error!("set main DPS cursor passthrough failed: {error}");
        CommandError::window_operation_failed()
    })?;
    state.set_passthrough(enabled);
    Ok(())
}

pub(crate) fn toggle_passthrough(
    window: &WebviewWindow,
    state: &AppState,
) -> Result<(), CommandError> {
    set_passthrough(window, state, !state.passthrough())
}

pub(crate) fn set_opacity(window: &WebviewWindow, opacity: f32) -> Result<(), CommandError> {
    let hwnd = window.hwnd().map_err(|error| {
        log::error!("read main DPS HWND failed: {error}");
        CommandError::window_operation_failed()
    })?;
    nte_dps_tool::platform::window_style::apply_uniform_opacity(hwnd.0 as isize, opacity).map_err(
        |error| {
            log::error!("set main DPS opacity failed: {error}");
            CommandError::window_operation_failed()
        },
    )
}

#[tauri::command]
pub(crate) fn show_main_dps_when_ready(window: WebviewWindow) -> Result<(), CommandError> {
    validate_window(&window)?;
    window.show().map_err(|error| {
        log::error!("show main DPS after the frontend first paint failed: {error}");
        CommandError::window_operation_failed()
    })?;
    window.unminimize().map_err(|error| {
        log::error!("restore main DPS after the frontend first paint failed: {error}");
        CommandError::window_operation_failed()
    })?;
    window.set_focus().map_err(|error| {
        log::error!("focus main DPS after the frontend first paint failed: {error}");
        CommandError::window_operation_failed()
    })
}

pub(crate) fn initialize(window: &WebviewWindow, app: AppHandle, state: AppState) {
    let webview = window.as_ref().clone();
    if let Err(error) = webview.set_auto_resize(true) {
        log::warn!("enable native main DPS WebView auto-resize failed: {error}");
    }
    if let Err(error) = restore_geometry(window, &state) {
        log::warn!("restore main DPS geometry failed: {error}");
    }
    if let Err(error) = set_opacity(window, state.ui_config_snapshot().opacity) {
        log::warn!("restore main DPS opacity failed: {error:?}");
    }
    let event_window = window.clone();
    window.on_window_event(move |event| {
        if matches!(event, WindowEvent::CloseRequested { .. }) {
            persist_geometry(&event_window, &state);
            for label in [
                HUD_WINDOW_LABEL,
                CONSOLE_WINDOW_LABEL,
                ABYSS_VALUES_WINDOW_LABEL,
                COMBAT_DETAILS_WINDOW_LABEL,
            ] {
                if let Some(owned) = app.get_webview_window(label)
                    && let Err(error) = owned.destroy()
                {
                    log::error!("destroy main DPS owned window {label} failed: {error}");
                }
            }
        }
    });
}

fn restore_geometry(window: &WebviewWindow, state: &AppState) -> Result<(), String> {
    let (size, position) = state.main_window_geometry();
    if let Some(size) = size {
        let [width, height] = migrate_legacy_default_size(size);
        window
            .set_size(Size::Logical(LogicalSize::new(
                f64::from(width.max(420.0)),
                f64::from(height.max(360.0)),
            )))
            .map_err(|error| error.to_string())?;
    }
    if let Some([x, y]) = restorable_main_window_position(position) {
        window
            .set_position(Position::Logical(LogicalPosition::new(
                f64::from(x),
                f64::from(y),
            )))
            .map_err(|error| error.to_string())?;
    } else {
        window.center().map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn restorable_main_window_position(position: Option<[f32; 2]>) -> Option<[f32; 2]> {
    let [x, y] = position?;
    (x.is_finite()
        && y.is_finite()
        && !(x <= MINIMIZED_POSITION_LIMIT && y <= MINIMIZED_POSITION_LIMIT))
        .then_some([x, y])
}

fn migrate_legacy_default_size(size: [f32; 2]) -> [f32; 2] {
    if (size[0] - LEGACY_TAURI_MAIN_DPS_DEFAULT_SIZE[0]).abs() < f32::EPSILON
        && (size[1] - LEGACY_TAURI_MAIN_DPS_DEFAULT_SIZE[1]).abs() < f32::EPSILON
    {
        COMPACT_TAURI_MAIN_DPS_DEFAULT_SIZE
    } else {
        size
    }
}

fn persist_geometry(window: &WebviewWindow, state: &AppState) {
    let minimized = window.is_minimized().unwrap_or(false);
    let maximized = window.is_maximized().unwrap_or(false);
    if !should_persist_geometry(minimized, maximized) {
        return;
    }
    let scale = window.scale_factor().unwrap_or(1.0).max(f64::EPSILON);
    let size = window.inner_size().ok().map(|value| {
        [
            (f64::from(value.width) / scale) as f32,
            (f64::from(value.height) / scale) as f32,
        ]
    });
    let position = window
        .outer_position()
        .ok()
        .map(|value| {
            [
                (f64::from(value.x) / scale) as f32,
                (f64::from(value.y) / scale) as f32,
            ]
        })
        .and_then(|position| restorable_main_window_position(Some(position)));
    if let Err(error) = state.set_main_window_geometry(size, position) {
        log::error!("persist main DPS geometry failed: {error}");
    }
}

fn should_persist_geometry(minimized: bool, maximized: bool) -> bool {
    !minimized && !maximized
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_reveal_fallback_only_targets_finished_main_page() {
        assert!(should_reveal_after_page_load(
            MAIN_DPS_WINDOW_LABEL,
            PageLoadEvent::Finished
        ));
        assert!(!should_reveal_after_page_load(
            MAIN_DPS_WINDOW_LABEL,
            PageLoadEvent::Started
        ));
        assert!(!should_reveal_after_page_load(
            CONSOLE_WINDOW_LABEL,
            PageLoadEvent::Finished
        ));
    }

    #[test]
    fn minimized_windows_sentinel_is_not_restored() {
        assert_eq!(
            restorable_main_window_position(Some([-16_000.0, -16_000.0])),
            None
        );
        assert_eq!(
            restorable_main_window_position(Some([-1920.0, 84.0])),
            Some([-1920.0, 84.0])
        );
        assert_eq!(
            restorable_main_window_position(Some([f32::NAN, 84.0])),
            None
        );
    }

    #[test]
    fn transient_minimized_or_maximized_geometry_is_not_persisted() {
        assert!(should_persist_geometry(false, false));
        assert!(!should_persist_geometry(true, false));
        assert!(!should_persist_geometry(false, true));
    }

    #[test]
    fn previous_tauri_default_height_moves_to_the_compact_baseline() {
        assert_eq!(
            migrate_legacy_default_size(LEGACY_TAURI_MAIN_DPS_DEFAULT_SIZE),
            COMPACT_TAURI_MAIN_DPS_DEFAULT_SIZE
        );
        assert_eq!(migrate_legacy_default_size([720.0, 640.0]), [720.0, 640.0]);
    }
}
