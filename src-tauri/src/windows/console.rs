use std::{sync::mpsc, thread, time::Duration};

use nte_dps_tool::storage::config::CONSOLE_WINDOW_MIN_SIZE;
use tauri::{LogicalPosition, LogicalSize, Position, Size, WebviewWindow, WindowEvent};

use crate::{contract::CommandError, state::AppState};

pub(crate) const CONSOLE_WINDOW_LABEL: &str = "console";
pub(crate) const CONSOLE_NAVIGATE_EVENT: &str = "console-navigate";
const GEOMETRY_SAVE_DEBOUNCE: Duration = Duration::from_millis(180);
const MINIMIZED_POSITION_LIMIT: f32 = -10_000.0;

#[derive(Clone, Copy, Debug, PartialEq)]
struct ConsoleGeometry {
    size: [f32; 2],
    position: [f32; 2],
}

pub(crate) fn validate_window(window: &WebviewWindow) -> Result<(), CommandError> {
    if window.label() == CONSOLE_WINDOW_LABEL {
        Ok(())
    } else {
        Err(CommandError::invalid_window())
    }
}

#[tauri::command]
pub(crate) fn show_console_when_ready(window: WebviewWindow) -> Result<(), CommandError> {
    validate_window(&window)?;
    window.show().map_err(|error| {
        log::error!("show Console after the frontend first paint failed: {error}");
        CommandError::window_operation_failed()
    })
}

/// The main DPS window owns the desktop lifetime. Closing Console only hides
/// it so toolbar state and subscriptions can be restored without rebuilding.
pub(crate) fn initialize(window: &WebviewWindow, state: &AppState) -> Result<(), String> {
    let console_webview = window.as_ref().clone();
    if let Err(error) = console_webview.set_auto_resize(true) {
        log::warn!("enable native Console WebView auto-resize failed: {error}");
    }
    restore_geometry(window, state)?;

    let (sender, receiver) = mpsc::channel::<Option<ConsoleGeometry>>();
    let persistence_state = state.clone();
    thread::Builder::new()
        .name("tauri-console-geometry-save".to_owned())
        .spawn(move || persist_geometry_loop(receiver, persistence_state))
        .map_err(|error| error.to_string())?;

    let close_window = window.clone();
    let event_window = window.clone();
    window.on_window_event(move |event| match event {
        WindowEvent::Moved(_)
        | WindowEvent::Resized(_)
        | WindowEvent::ScaleFactorChanged { .. } => {
            if let Some(geometry) = current_geometry(&event_window) {
                let _ = sender.send(Some(geometry));
            }
        }
        WindowEvent::CloseRequested { api, .. } => {
            api.prevent_close();
            if let Some(geometry) = current_geometry(&event_window) {
                let _ = sender.send(Some(geometry));
            }
            if let Err(error) = close_window.hide() {
                log::error!("hide Console window failed: {error}");
            }
        }
        WindowEvent::Destroyed => {
            let _ = sender.send(None);
        }
        _ => {}
    });
    Ok(())
}

fn restore_geometry(window: &WebviewWindow, state: &AppState) -> Result<(), String> {
    let (size, position) = state.console_window_geometry();
    if let Some([width, height]) = size {
        window
            .set_size(Size::Logical(LogicalSize::new(
                f64::from(width.max(CONSOLE_WINDOW_MIN_SIZE[0])),
                f64::from(height.max(CONSOLE_WINDOW_MIN_SIZE[1])),
            )))
            .map_err(|error| error.to_string())?;
    }
    if let Some([x, y]) = restorable_position(position) {
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

fn current_geometry(window: &WebviewWindow) -> Option<ConsoleGeometry> {
    if window.is_minimized().unwrap_or(false) || window.is_maximized().unwrap_or(false) {
        return None;
    }
    let scale = window.scale_factor().unwrap_or(1.0).max(f64::EPSILON);
    let size = window.inner_size().ok()?;
    let position = window.outer_position().ok()?;
    let position = restorable_position(Some([
        (f64::from(position.x) / scale) as f32,
        (f64::from(position.y) / scale) as f32,
    ]))?;
    Some(ConsoleGeometry {
        size: [
            (f64::from(size.width) / scale) as f32,
            (f64::from(size.height) / scale) as f32,
        ],
        position,
    })
}

fn persist_geometry_loop(receiver: mpsc::Receiver<Option<ConsoleGeometry>>, state: AppState) {
    while let Ok(message) = receiver.recv() {
        let Some(mut latest) = message else {
            return;
        };
        loop {
            match receiver.recv_timeout(GEOMETRY_SAVE_DEBOUNCE) {
                Ok(Some(next)) => latest = next,
                Ok(None) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                    persist_geometry(&state, latest);
                    return;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => break,
            }
        }
        persist_geometry(&state, latest);
    }
}

fn persist_geometry(state: &AppState, geometry: ConsoleGeometry) {
    if let Err(error) = state.set_console_window_geometry(geometry.size, geometry.position) {
        log::error!("persist Console window geometry failed: {error}");
    }
}

fn restorable_position(position: Option<[f32; 2]>) -> Option<[f32; 2]> {
    let [x, y] = position?;
    (x.is_finite()
        && y.is_finite()
        && !(x <= MINIMIZED_POSITION_LIMIT && y <= MINIMIZED_POSITION_LIMIT))
        .then_some([x, y])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_secondary_monitor_positions_remain_restorable() {
        assert_eq!(
            restorable_position(Some([-1920.0, 84.0])),
            Some([-1920.0, 84.0])
        );
        assert_eq!(restorable_position(Some([-16_000.0, -16_000.0])), None);
        assert_eq!(restorable_position(Some([f32::NAN, 84.0])), None);
    }
}
