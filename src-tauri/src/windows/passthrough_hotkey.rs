use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, RecvTimeoutError},
    },
    thread,
    time::Duration,
};

use nte_dps_tool::platform::passthrough_hotkey::{PassthroughHotkeyEvent, PassthroughHotkeyHandle};
use nte_dps_tool::{core::live_capture::LiveCapturePhase, storage::config::GlobalHotkeyAction};
use tauri::{AppHandle, Emitter, Manager};

use crate::{
    state::AppState,
    windows::{hud, main_dps},
};

const HOTKEY_EVENT_POLL_INTERVAL: Duration = Duration::from_millis(50);

pub(crate) struct HudPassthroughHotkeyRuntime {
    stop: Arc<AtomicBool>,
    dispatcher: Option<thread::JoinHandle<()>>,
    _listener: PassthroughHotkeyHandle,
}

impl HudPassthroughHotkeyRuntime {
    pub(crate) fn start(app: AppHandle, state: AppState) -> Result<Self, String> {
        let (listener, receiver) =
            PassthroughHotkeyHandle::start(state.passthrough_hotkey(), state.global_hotkeys())?;
        let stop = Arc::new(AtomicBool::new(false));
        let dispatcher_stop = Arc::clone(&stop);
        let dispatcher = thread::Builder::new()
            .name("tauri-hud-hotkey-dispatch".to_owned())
            .spawn(move || run_dispatcher(app, state, receiver, dispatcher_stop))
            .map_err(|error| error.to_string())?;

        Ok(Self {
            stop,
            dispatcher: Some(dispatcher),
            _listener: listener,
        })
    }
}

impl Drop for HudPassthroughHotkeyRuntime {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(dispatcher) = self.dispatcher.take() {
            let _ = dispatcher.join();
        }
    }
}

fn run_dispatcher(
    app: AppHandle,
    state: AppState,
    receiver: Receiver<PassthroughHotkeyEvent>,
    stop: Arc<AtomicBool>,
) {
    while !stop.load(Ordering::Acquire) {
        match receiver.recv_timeout(HOTKEY_EVENT_POLL_INTERVAL) {
            Ok(PassthroughHotkeyEvent::HookInstalled) => {
                state.set_passthrough_hotkey_ready(true);
            }
            Ok(PassthroughHotkeyEvent::HookInstallFailed { error }) => {
                state.set_passthrough_hotkey_ready(false);
                log::error!("install HUD passthrough hotkey hook failed with error {error}");
                restore_editing_after_hook_failure(&app, &state);
            }
            Ok(PassthroughHotkeyEvent::Toggle) => {
                if let Some(main_window) = app.get_webview_window(main_dps::MAIN_DPS_WINDOW_LABEL)
                    && main_window.is_visible().unwrap_or(false)
                {
                    if let Err(error) = main_dps::toggle_passthrough(&main_window, &state) {
                        log::error!("toggle main DPS passthrough from hotkey failed: {error:?}");
                    }
                    continue;
                }
                let Some(window) = app.get_webview_window(hud::HUD_WINDOW_LABEL) else {
                    continue;
                };
                if let Err(error) = hud::toggle_passthrough(&window, &state) {
                    log::error!("toggle HUD passthrough from hotkey failed: {error:?}");
                }
            }
            Ok(PassthroughHotkeyEvent::GlobalAction(action)) => {
                dispatch_global_action(&app, &state, action);
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

fn dispatch_global_action(app: &AppHandle, state: &AppState, action: GlobalHotkeyAction) {
    match action {
        GlobalHotkeyAction::ToggleCapture => {
            let phase = state.capture_phase();
            if matches!(
                phase,
                LiveCapturePhase::Idle | LiveCapturePhase::Stopped | LiveCapturePhase::Failed
            ) && state.session_has_data()
            {
                request_main_confirmation(app, "start");
                return;
            }
            let result = match phase {
                LiveCapturePhase::Starting
                | LiveCapturePhase::Running
                | LiveCapturePhase::Stopping => state.request_capture_stop(),
                LiveCapturePhase::Idle | LiveCapturePhase::Stopped | LiveCapturePhase::Failed => {
                    state.request_capture_start(false)
                }
            };
            if let Err(error) = result {
                log::error!("toggle capture from global hotkey failed: {}", error.detail);
            }
        }
        GlobalHotkeyAction::ResetSession => {
            let active = matches!(
                state.capture_phase(),
                LiveCapturePhase::Starting | LiveCapturePhase::Running | LiveCapturePhase::Stopping
            ) || state.replay_running();
            if active {
                request_main_confirmation(app, "reset");
            } else {
                let undo_token = state.reset_session_with_undo();
                state.publish_island_notice(
                    "success",
                    if undo_token.is_some() {
                        "Session reset · use Undo within 5 seconds"
                    } else {
                        "Stats reset"
                    },
                    Vec::new(),
                    undo_token,
                );
                let _ = super::island::show_notice(app, state);
            }
        }
        GlobalHotkeyAction::ToggleHud => {
            let Some(window) = app.get_webview_window(hud::HUD_WINDOW_LABEL) else {
                return;
            };
            match window.is_visible() {
                Ok(true) => {
                    if let Err(error) = window.hide() {
                        log::error!("hide HUD from global hotkey failed: {error}");
                    }
                }
                Ok(false) => {
                    if let Err(error) = window.show() {
                        log::error!("show HUD from global hotkey failed: {error}");
                    }
                }
                Err(error) => {
                    log::error!("query HUD visibility from global hotkey failed: {error}")
                }
            }
        }
    }
}

fn request_main_confirmation(app: &AppHandle, action: &'static str) {
    let Some(window) = app.get_webview_window(main_dps::MAIN_DPS_WINDOW_LABEL) else {
        return;
    };
    if let Err(error) = window.show() {
        log::error!("show main DPS confirmation window failed: {error}");
        return;
    }
    let _ = window.unminimize();
    let _ = window.set_focus();
    if let Err(error) = window.emit(main_dps::MAIN_DPS_CONFIRMATION_EVENT, action) {
        log::error!("emit main DPS confirmation request failed: {error}");
    }
}

fn restore_editing_after_hook_failure(app: &AppHandle, state: &AppState) {
    if !state.passthrough() {
        return;
    }
    if let Some(window) = app.get_webview_window(main_dps::MAIN_DPS_WINDOW_LABEL)
        && window.is_visible().unwrap_or(false)
    {
        if let Err(error) = main_dps::set_passthrough(&window, state, false) {
            log::error!("restore main DPS editing after hotkey hook failure failed: {error:?}");
        }
    } else if let Some(window) = app.get_webview_window(hud::HUD_WINDOW_LABEL)
        && let Err(error) = hud::set_passthrough(&window, state, false)
    {
        log::error!("restore HUD editing after hotkey hook failure failed: {error:?}");
    }
}
