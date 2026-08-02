use std::{sync::mpsc, thread, time::Duration};

use tauri::{Emitter, LogicalPosition, WebviewWindow, WindowEvent};

use crate::{contract::CommandError, state::AppState};

pub(crate) const COMBAT_DETAILS_WINDOW_LABEL: &str = "combat-details";
pub(crate) const COMBAT_DETAILS_CHANGED_EVENT: &str = "main-dps-detail-requested";

pub(crate) fn validate_window(window: &WebviewWindow) -> Result<(), CommandError> {
    if window.label() == COMBAT_DETAILS_WINDOW_LABEL {
        Ok(())
    } else {
        Err(CommandError::invalid_window())
    }
}

pub(crate) fn show(window: &WebviewWindow, state: &AppState) -> Result<(), CommandError> {
    if let Some([x, y]) = state.main_dps_detail_window_position()
        && x.is_finite()
        && y.is_finite()
    {
        window
            .set_position(LogicalPosition::new(f64::from(x), f64::from(y)))
            .map_err(window_error)?;
    }
    window.show().map_err(window_error)?;
    window.unminimize().map_err(window_error)?;
    window.set_focus().map_err(window_error)?;
    window
        .emit(COMBAT_DETAILS_CHANGED_EVENT, ())
        .map_err(window_error)
}

pub(crate) fn initialize(window: &WebviewWindow, state: &AppState) -> Result<(), String> {
    let webview = window.as_ref().clone();
    if let Err(error) = webview.set_auto_resize(true) {
        log::warn!("enable native combat details WebView auto-resize failed: {error}");
    }
    let (sender, receiver) = mpsc::channel::<Option<([f32; 2], bool)>>();
    let persistence_state = state.clone();
    thread::Builder::new()
        .name("tauri-combat-detail-position-save".to_owned())
        .spawn(move || {
            while let Ok(Some(mut latest)) = receiver.recv() {
                loop {
                    match receiver.recv_timeout(Duration::from_millis(180)) {
                        Ok(Some(next)) => latest = next,
                        Ok(None) | Err(mpsc::RecvTimeoutError::Disconnected) => return,
                        Err(mpsc::RecvTimeoutError::Timeout) => break,
                    }
                }
                if let Err(error) =
                    persistence_state.set_main_dps_detail_window_position(latest.0, latest.1)
                {
                    log::error!("persist combat detail window position failed: {error}");
                }
            }
        })
        .map_err(|error| error.to_string())?;

    let close_window = window.clone();
    let event_window = window.clone();
    let event_state = state.clone();
    window.on_window_event(move |event| match event {
        WindowEvent::Moved(position) => {
            let scale = event_window.scale_factor().unwrap_or(1.0);
            let logical = position.to_logical::<f32>(scale);
            let character = event_state.main_dps_detail_request().character_id.is_some();
            let _ = sender.send(Some(([logical.x, logical.y], character)));
        }
        WindowEvent::CloseRequested { api, .. } => {
            api.prevent_close();
            if let Err(error) = close_window.hide() {
                log::error!("hide combat details window failed: {error}");
            }
        }
        WindowEvent::Destroyed => {
            let _ = sender.send(None);
        }
        _ => {}
    });
    Ok(())
}

fn window_error(error: tauri::Error) -> CommandError {
    log::error!("combat details window operation failed: {error}");
    CommandError::window_operation_failed()
}
