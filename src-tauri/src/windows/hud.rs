use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError},
    },
    thread,
    time::Duration,
};

use tauri::{LogicalSize, Size, WebviewWindow, WindowEvent};

use nte_dps_tool::storage::config::{HUD_WIDTH_MAX, HUD_WIDTH_MIN};

use crate::{
    contract::CommandError, settings_service::SettingsServiceError, state::AppState,
    windows::window_position,
};

pub(crate) const HUD_WINDOW_LABEL: &str = "hud-spike";
// Keep native edge-resize persistence aligned with the shared config debounce.
const HUD_WIDTH_SAVE_DELAY: Duration = Duration::from_millis(350);
const HUD_POSITION_SAVE_DELAY: Duration = Duration::from_millis(350);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DebouncedEvent<T> {
    Changed(T),
    Shutdown,
}

type NativeWidthEvent = DebouncedEvent<u16>;
type NativePositionEvent = DebouncedEvent<[i32; 2]>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct HudContentConstraints {
    min_width: u16,
    max_width: u16,
    height: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HeightConstraintOrder {
    MinThenMax,
    MaxThenMin,
}

pub(crate) fn validate_window(window: &WebviewWindow) -> Result<(), CommandError> {
    if window.label() == HUD_WINDOW_LABEL {
        Ok(())
    } else {
        Err(CommandError::invalid_window())
    }
}

pub(crate) fn set_editing_effect(window: &WebviewWindow, enabled: bool) -> Result<(), String> {
    let hwnd = window.hwnd().map_err(|error| error.to_string())?;
    nte_dps_tool::platform::window_style::apply_persistent_acrylic(hwnd.0 as isize, enabled)
}

pub(crate) fn set_native_shape(window: &WebviewWindow) -> Result<(), String> {
    let hwnd = window.hwnd().map_err(|error| error.to_string())?;
    nte_dps_tool::platform::window_style::apply_borderless_rounding(hwnd.0 as isize)
        .map_err(|result| format!("DwmSetWindowAttribute failed with HRESULT {result:#010x}"))
}

pub(crate) fn initialize_content_size(
    window: &WebviewWindow,
    state: &AppState,
) -> tauri::Result<()> {
    let constraints = hud_content_constraints(state.hud_initial_height());
    window.set_size(Size::Logical(LogicalSize::new(
        f64::from(state.hud_width()),
        f64::from(constraints.height),
    )))?;
    set_height_constraints(window, constraints, HeightConstraintOrder::MinThenMax)
}

pub(crate) fn restore_content_position(
    window: &WebviewWindow,
    state: &AppState,
) -> Result<(), String> {
    let Some(saved_position) = state.hud_window_position() else {
        return Ok(());
    };
    window_position::restore_window_position(window, saved_position)
}

pub(crate) fn track_native_width(window: &WebviewWindow, state: &AppState) -> Result<(), String> {
    let scale_factor = window.scale_factor().map_err(|error| error.to_string())?;
    let scale_factor_bits = Arc::new(AtomicU64::new(scale_factor.to_bits()));
    let (sender, receiver) = mpsc::channel();
    let persistence_state = state.clone();
    thread::Builder::new()
        .name("tauri-hud-width-save".to_owned())
        .spawn(move || {
            run_debounced_persistence(receiver, HUD_WIDTH_SAVE_DELAY, |width| {
                if let Err(error) = persistence_state.set_hud_width(width) {
                    log::error!("persist native HUD width failed: {error}");
                }
            });
        })
        .map_err(|error| error.to_string())?;

    let event_scale_factor_bits = Arc::clone(&scale_factor_bits);
    window.on_window_event(move |event| match event {
        WindowEvent::Resized(size) => {
            let scale_factor = f64::from_bits(event_scale_factor_bits.load(Ordering::Acquire));
            if let Some(width) = logical_hud_width(size.width, scale_factor) {
                let _ = sender.send(NativeWidthEvent::Changed(width));
            }
        }
        WindowEvent::ScaleFactorChanged {
            scale_factor,
            new_inner_size,
            ..
        } => {
            event_scale_factor_bits.store(scale_factor.to_bits(), Ordering::Release);
            if let Some(width) = logical_hud_width(new_inner_size.width, *scale_factor) {
                let _ = sender.send(NativeWidthEvent::Changed(width));
            }
        }
        WindowEvent::Destroyed => {
            let _ = sender.send(NativeWidthEvent::Shutdown);
        }
        _ => {}
    });
    Ok(())
}

pub(crate) fn track_native_position(
    window: &WebviewWindow,
    state: &AppState,
) -> Result<(), String> {
    let (sender, receiver) = mpsc::channel();
    let persistence_state = state.clone();
    thread::Builder::new()
        .name("tauri-hud-position-save".to_owned())
        .spawn(move || {
            run_debounced_persistence(receiver, HUD_POSITION_SAVE_DELAY, |position| {
                if let Err(error) = persistence_state.set_hud_window_position(position) {
                    log::error!("persist native HUD position failed: {error}");
                }
            });
        })
        .map_err(|error| error.to_string())?;

    window.on_window_event(move |event| match event {
        WindowEvent::Moved(position) => {
            let _ = sender.send(NativePositionEvent::Changed([position.x, position.y]));
        }
        WindowEvent::Destroyed => {
            let _ = sender.send(NativePositionEvent::Shutdown);
        }
        _ => {}
    });
    Ok(())
}

fn logical_hud_width(physical_width: u32, scale_factor: f64) -> Option<u16> {
    if !scale_factor.is_finite() || scale_factor <= 0.0 {
        return None;
    }
    let width = (f64::from(physical_width) / scale_factor)
        .round()
        .clamp(f64::from(HUD_WIDTH_MIN), f64::from(HUD_WIDTH_MAX));
    Some(width as u16)
}

fn run_debounced_persistence<T: Copy>(
    receiver: Receiver<DebouncedEvent<T>>,
    save_delay: Duration,
    mut persist: impl FnMut(T),
) {
    loop {
        let mut pending = match receiver.recv() {
            Ok(DebouncedEvent::Changed(value)) => value,
            Ok(DebouncedEvent::Shutdown) | Err(_) => return,
        };

        loop {
            match receiver.recv_timeout(save_delay) {
                Ok(DebouncedEvent::Changed(value)) => pending = value,
                Ok(DebouncedEvent::Shutdown) | Err(RecvTimeoutError::Disconnected) => {
                    persist(pending);
                    return;
                }
                Err(RecvTimeoutError::Timeout) => {
                    persist(pending);
                    break;
                }
            }
        }
    }
}

pub(crate) fn sync_content_height(
    window: &WebviewWindow,
    state: &AppState,
) -> Result<(), CommandError> {
    let current = logical_inner_size(window)?;
    resize_with_locked_height(
        window,
        current.width,
        current.height,
        state.hud_initial_height(),
    )
    .map_err(|error| {
        log::error!("synchronize HUD content height failed: {error}");
        CommandError::window_operation_failed()
    })
}

pub(crate) fn sync_content_width(
    window: &WebviewWindow,
    state: &AppState,
) -> Result<(), CommandError> {
    let current = logical_inner_size(window)?;
    resize_with_locked_height(
        window,
        f64::from(state.hud_width()),
        current.height,
        state.hud_initial_height(),
    )
    .map_err(|error| {
        log::error!("synchronize HUD content width failed: {error}");
        CommandError::window_operation_failed()
    })
}

fn logical_inner_size(window: &WebviewWindow) -> Result<LogicalSize<f64>, CommandError> {
    let scale_factor = window.scale_factor().map_err(|error| {
        log::error!("read HUD scale factor failed: {error}");
        CommandError::window_operation_failed()
    })?;
    window
        .inner_size()
        .map(|size| size.to_logical::<f64>(scale_factor))
        .map_err(|error| {
            log::error!("read HUD inner size failed: {error}");
            CommandError::window_operation_failed()
        })
}

fn resize_with_locked_height(
    window: &WebviewWindow,
    width: f64,
    current_height: f64,
    target_height: u16,
) -> tauri::Result<()> {
    let constraints = hud_content_constraints(target_height);
    set_height_constraints(
        window,
        constraints,
        height_constraint_order(current_height, target_height),
    )?;
    window.set_size(Size::Logical(LogicalSize::new(
        width,
        f64::from(target_height),
    )))
}

fn set_height_constraints(
    window: &WebviewWindow,
    constraints: HudContentConstraints,
    order: HeightConstraintOrder,
) -> tauri::Result<()> {
    let min_size = LogicalSize::new(
        f64::from(constraints.min_width),
        f64::from(constraints.height),
    );
    let max_size = LogicalSize::new(
        f64::from(constraints.max_width),
        f64::from(constraints.height),
    );
    match order {
        HeightConstraintOrder::MinThenMax => {
            window.set_min_size(Some(min_size))?;
            window.set_max_size(Some(max_size))?;
        }
        HeightConstraintOrder::MaxThenMin => {
            window.set_max_size(Some(max_size))?;
            window.set_min_size(Some(min_size))?;
        }
    }
    Ok(())
}

fn hud_content_constraints(height: u16) -> HudContentConstraints {
    HudContentConstraints {
        min_width: HUD_WIDTH_MIN,
        max_width: HUD_WIDTH_MAX,
        height,
    }
}

fn height_constraint_order(current_height: f64, target_height: u16) -> HeightConstraintOrder {
    if f64::from(target_height) > current_height {
        HeightConstraintOrder::MaxThenMin
    } else {
        HeightConstraintOrder::MinThenMax
    }
}

pub(crate) fn set_passthrough(
    window: &WebviewWindow,
    state: &AppState,
    enabled: bool,
) -> Result<(), CommandError> {
    let _transaction = state
        .lock_passthrough_transaction()
        .map_err(passthrough_transaction_error)?;
    set_passthrough_locked(window, state, enabled)
}

pub(crate) fn toggle_passthrough(
    window: &WebviewWindow,
    state: &AppState,
) -> Result<(), CommandError> {
    let _transaction = state
        .lock_passthrough_transaction()
        .map_err(passthrough_transaction_error)?;
    set_passthrough_locked(window, state, !state.passthrough())
}

fn set_passthrough_locked(
    window: &WebviewWindow,
    state: &AppState,
    enabled: bool,
) -> Result<(), CommandError> {
    if enabled && !state.passthrough_hotkey_ready() {
        return Err(CommandError::passthrough_hotkey_unavailable());
    }
    window.set_ignore_cursor_events(enabled).map_err(|error| {
        log::error!("set_ignore_cursor_events failed: {error}");
        CommandError::window_operation_failed()
    })?;
    if let Err(error) = set_editing_effect(window, !enabled) {
        log::warn!("native Acrylic failed while changing HUD passthrough: {error}");
    }
    if let Err(error) = set_native_shape(window) {
        log::warn!("native HUD shape refresh failed: {error}");
    }
    state.set_passthrough(enabled);
    if sync_content_height(window, state).is_err() {
        log::warn!("native HUD height refresh failed while changing passthrough");
    }
    Ok(())
}

fn passthrough_transaction_error(_error: SettingsServiceError) -> CommandError {
    CommandError::passthrough_state_unavailable()
}

pub(crate) fn set_always_on_top(
    window: &WebviewWindow,
    state: &AppState,
    enabled: bool,
) -> Result<(), CommandError> {
    let previous = state.always_on_top();
    window.set_always_on_top(enabled).map_err(|error| {
        log::error!("set_always_on_top failed: {error}");
        CommandError::window_operation_failed()
    })?;
    if let Err(error) = state.set_always_on_top(enabled) {
        log::error!("save HUD always-on-top preference failed: {error}");
        if let Err(rollback_error) = window.set_always_on_top(previous) {
            log::error!("restore HUD always-on-top state failed: {rollback_error}");
        }
        return Err(CommandError::hud_config_save_failed());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hud_constraints_keep_width_resizable_and_lock_content_height() {
        assert_eq!(
            hud_content_constraints(260),
            HudContentConstraints {
                min_width: HUD_WIDTH_MIN,
                max_width: HUD_WIDTH_MAX,
                height: 260,
            }
        );
    }

    #[test]
    fn height_constraint_updates_expand_the_max_before_the_minimum() {
        assert_eq!(
            height_constraint_order(180.0, 260),
            HeightConstraintOrder::MaxThenMin
        );
        assert_eq!(
            height_constraint_order(260.0, 180),
            HeightConstraintOrder::MinThenMax
        );
        assert_eq!(
            height_constraint_order(260.0, 260),
            HeightConstraintOrder::MinThenMax
        );
    }

    #[test]
    fn physical_resize_width_is_rounded_in_logical_pixels_and_clamped() {
        assert_eq!(logical_hud_width(760, 2.0), Some(380));
        assert_eq!(logical_hud_width(559, 2.0), Some(HUD_WIDTH_MIN));
        assert_eq!(logical_hud_width(8_000, 2.0), Some(HUD_WIDTH_MAX));
        assert_eq!(logical_hud_width(760, 0.0), None);
        assert_eq!(logical_hud_width(760, f64::NAN), None);
    }

    #[test]
    fn native_resize_persistence_coalesces_to_the_latest_width() {
        let (sender, receiver) = mpsc::channel();
        sender
            .send(NativeWidthEvent::Changed(420))
            .expect("queue first width");
        sender
            .send(NativeWidthEvent::Changed(512))
            .expect("queue latest width");
        sender
            .send(NativeWidthEvent::Shutdown)
            .expect("queue shutdown");

        let mut persisted = Vec::new();
        run_debounced_persistence(receiver, Duration::from_secs(1), |width| {
            persisted.push(width);
        });

        assert_eq!(persisted, vec![512]);
    }

    #[test]
    fn native_resize_persistence_flushes_after_the_quiet_period() {
        let (sender, receiver) = mpsc::channel();
        sender
            .send(NativeWidthEvent::Changed(420))
            .expect("queue width");
        let shutdown_sender = sender.clone();

        let mut persisted = Vec::new();
        run_debounced_persistence(receiver, Duration::ZERO, |width| {
            persisted.push(width);
            shutdown_sender
                .send(NativeWidthEvent::Shutdown)
                .expect("queue shutdown after persistence");
        });

        assert_eq!(persisted, vec![420]);
    }
}
