use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewWindow, WindowEvent,
    window::Monitor,
};

use crate::{contract::CommandError, state::AppState};

use super::{
    abyss_values::ABYSS_VALUES_WINDOW_LABEL,
    combat_details::{CHARACTER_DETAILS_WINDOW_LABEL, TEAM_DETAILS_WINDOW_LABEL},
    console::CONSOLE_WINDOW_LABEL,
    hud::HUD_WINDOW_LABEL,
    main_dps::MAIN_DPS_WINDOW_LABEL,
};

pub(crate) const ISLAND_WINDOW_LABEL: &str = "notification-island";
pub(crate) const ISLAND_CHANGED_EVENT: &str = "notification-island-changed";
const ISLAND_LOGICAL_WIDTH: f64 = 440.0;
const ISLAND_LOGICAL_HEIGHT: f64 = 76.0;

const NOTICE_SOURCE_WINDOW_LABELS: [&str; 6] = [
    MAIN_DPS_WINDOW_LABEL,
    HUD_WINDOW_LABEL,
    CONSOLE_WINDOW_LABEL,
    ABYSS_VALUES_WINDOW_LABEL,
    CHARACTER_DETAILS_WINDOW_LABEL,
    TEAM_DETAILS_WINDOW_LABEL,
];

pub(crate) fn validate_window(window: &WebviewWindow) -> Result<(), CommandError> {
    if window.label() == ISLAND_WINDOW_LABEL {
        Ok(())
    } else {
        Err(CommandError::invalid_window())
    }
}

pub(crate) fn show_notice(app: &AppHandle, state: &AppState) -> Result<(), CommandError> {
    if !state.ui_config_snapshot().island_notifications {
        return Ok(());
    }
    let window = app
        .get_webview_window(ISLAND_WINDOW_LABEL)
        .ok_or_else(CommandError::window_operation_failed)?;
    if let Some(monitor) = notice_monitor(app, &window) {
        let size = island_physical_size(monitor.scale_factor());
        window
            .set_size(size)
            .map_err(|_| CommandError::window_operation_failed())?;
        let position = island_physical_position(
            *monitor.position(),
            *monitor.size(),
            size,
            monitor.scale_factor(),
            state.ui_config_snapshot().island_offset_x,
        );
        window
            .set_position(position)
            .map_err(|_| CommandError::window_operation_failed())?;
    }
    window
        .show()
        .map_err(|_| CommandError::window_operation_failed())?;
    window
        .emit(ISLAND_CHANGED_EVENT, ())
        .map_err(|_| CommandError::window_operation_failed())
}

fn island_physical_size(scale: f64) -> PhysicalSize<u32> {
    PhysicalSize::new(
        (ISLAND_LOGICAL_WIDTH * scale).round() as u32,
        (ISLAND_LOGICAL_HEIGHT * scale).round() as u32,
    )
}

fn island_physical_position(
    monitor_position: PhysicalPosition<i32>,
    monitor_size: PhysicalSize<u32>,
    window_size: PhysicalSize<u32>,
    scale: f64,
    logical_offset_x: f32,
) -> PhysicalPosition<i32> {
    let centered_x = f64::from(monitor_position.x)
        + (f64::from(monitor_size.width) - f64::from(window_size.width)) * 0.5
        + f64::from(logical_offset_x) * scale;
    let screen_margin = 12.0 * scale;
    let minimum_x = f64::from(monitor_position.x) + screen_margin;
    let maximum_x = f64::from(monitor_position.x) + f64::from(monitor_size.width)
        - f64::from(window_size.width)
        - screen_margin;
    let x = if maximum_x >= minimum_x {
        centered_x.clamp(minimum_x, maximum_x)
    } else {
        f64::from(monitor_position.x)
    };
    let y = f64::from(monitor_position.y) + 10.0 * scale;
    PhysicalPosition::new(x.round() as i32, y.round() as i32)
}

fn notice_monitor(app: &AppHandle, island_window: &WebviewWindow) -> Option<Monitor> {
    for label in NOTICE_SOURCE_WINDOW_LABELS {
        let Some(window) = app.get_webview_window(label) else {
            continue;
        };
        if window.is_focused().unwrap_or(false)
            && let Ok(Some(monitor)) = window.current_monitor()
        {
            return Some(monitor);
        }
    }

    for label in NOTICE_SOURCE_WINDOW_LABELS {
        let Some(window) = app.get_webview_window(label) else {
            continue;
        };
        if window.is_visible().unwrap_or(false)
            && let Ok(Some(monitor)) = window.current_monitor()
        {
            return Some(monitor);
        }
    }

    island_window
        .primary_monitor()
        .ok()
        .flatten()
        .or_else(|| island_window.current_monitor().ok().flatten())
}

pub(crate) fn bind_close_to_hide(window: &WebviewWindow) {
    let window = window.clone();
    window.clone().on_window_event(move |event| {
        if let WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            let _ = window.hide();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{island_physical_position, island_physical_size};
    use tauri::{PhysicalPosition, PhysicalSize};

    #[test]
    fn centers_island_against_the_monitor_not_an_app_window() {
        let position = island_physical_position(
            PhysicalPosition::new(0, 0),
            PhysicalSize::new(1920, 1080),
            PhysicalSize::new(440, 76),
            1.0,
            0.0,
        );

        assert_eq!(position, PhysicalPosition::new(740, 10));
    }

    #[test]
    fn preserves_logical_island_size_across_monitor_dpi() {
        assert_eq!(island_physical_size(1.0), PhysicalSize::new(440, 76));
        assert_eq!(island_physical_size(1.5), PhysicalSize::new(660, 114));
    }

    #[test]
    fn keeps_secondary_monitor_origin_and_dpi_scale() {
        let position = island_physical_position(
            PhysicalPosition::new(-2560, 120),
            PhysicalSize::new(2560, 1440),
            PhysicalSize::new(660, 114),
            1.5,
            20.0,
        );

        assert_eq!(position, PhysicalPosition::new(-1580, 135));
    }

    #[test]
    fn clamps_large_offsets_inside_screen_edges() {
        let position = island_physical_position(
            PhysicalPosition::new(0, 0),
            PhysicalSize::new(1920, 1080),
            PhysicalSize::new(440, 76),
            1.0,
            5000.0,
        );

        assert_eq!(position, PhysicalPosition::new(1468, 10));
    }
}
