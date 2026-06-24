use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows_sys::Win32::Foundation::{HWND, LPARAM};
use windows_sys::Win32::Graphics::Dwm::{
    DWMWA_BORDER_COLOR, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmSetWindowAttribute,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GWL_EXSTYLE, GetWindowLongPtrW, GetWindowThreadProcessId, HWND_NOTOPMOST,
    HWND_TOPMOST, IsWindowVisible, LWA_ALPHA, LWA_COLORKEY, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    SetLayeredWindowAttributes, SetWindowLongPtrW, SetWindowPos, WS_EX_LAYERED, WS_EX_TRANSPARENT,
};

/// DWM border-color sentinels (see `DWMWA_BORDER_COLOR`): suppress the 1px window
/// border for a true overlay, or restore the system default.
const DWMWA_COLOR_NONE: u32 = 0xFFFF_FFFE;
const DWMWA_COLOR_DEFAULT: u32 = 0xFFFF_FFFF;

/// Grey value the HUD clears its empty pixels to; `LWA_COLORKEY` then makes
/// exactly this colour transparent *and* click-through. The app paints this same
/// value as its clear colour in HUD mode (see `DpsApp::clear_color`). Kept very
/// dark and off pure-black so text/avatar pixels never accidentally match it, and
/// so anti-aliased fringes blend toward the dark halo rather than a bright key.
pub(crate) const HUD_CHROMA_KEY_RGB: u8 = 2;

/// `HUD_CHROMA_KEY_RGB` as a Win32 `COLORREF` (`0x00BBGGRR`).
fn hud_chroma_key_colorref() -> u32 {
    let value = HUD_CHROMA_KEY_RGB as u32;
    value | (value << 8) | (value << 16)
}

pub(crate) fn apply_window_attributes(
    frame: &eframe::Frame,
    opacity: f32,
    force_opacity: bool,
    applied_opacity: &mut Option<f32>,
    corner_applied_hwnd: &mut Option<isize>,
    hud_overlay: bool,
    passthrough: bool,
) {
    let opacity = opacity.clamp(0.35, 1.0);
    let Ok(window_handle) = frame.window_handle() else {
        return;
    };
    let RawWindowHandle::Win32(window_handle) = window_handle.as_raw() else {
        return;
    };
    let hwnd = window_handle.hwnd.get() as HWND;
    let hwnd_key = hwnd as isize;
    // SAFETY: hwnd comes from the active eframe Win32 window handle. The DWM
    // attribute pointers reference local constants for the duration of each call.
    unsafe {
        if *corner_applied_hwnd != Some(hwnd_key) {
            DwmSetWindowAttribute(
                hwnd,
                DWMWA_WINDOW_CORNER_PREFERENCE as u32,
                std::ptr::from_ref(&DWMWCP_ROUND).cast(),
                std::mem::size_of_val(&DWMWCP_ROUND) as u32,
            );
            *corner_applied_hwnd = Some(hwnd_key);
        }

        // Click-through to the game beneath requires WS_EX_TRANSPARENT *on a
        // layered window*. Testing showed WS_EX_TRANSPARENT alone (non-layered)
        // does NOT pass clicks to a separate-process game, and WM_NCHITTEST→
        // HTTRANSPARENT only cascades within one thread. So both modes stay
        // layered; the HUD gets its transparency from a colour key instead.
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let mut new_style = style | WS_EX_LAYERED as isize;
        if passthrough {
            new_style |= WS_EX_TRANSPARENT as isize;
        } else {
            new_style &= !(WS_EX_TRANSPARENT as isize);
        }
        let style_changed = new_style != style;
        if style_changed {
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, new_style);
        }

        // No window border in HUD mode so it reads as a true overlay, not a frame.
        let border = if hud_overlay {
            DWMWA_COLOR_NONE
        } else {
            DWMWA_COLOR_DEFAULT
        };
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_BORDER_COLOR as u32,
            std::ptr::from_ref(&border).cast(),
            std::mem::size_of_val(&border) as u32,
        );

        if hud_overlay {
            // A layered GL window can't do per-pixel alpha (UpdateLayeredWindow is
            // incompatible with the swapchain). So the HUD clears its empty pixels
            // to HUD_CHROMA_KEY_RGB and LWA_COLORKEY makes exactly that colour
            // transparent and click-through, while painted content stays opaque.
            SetLayeredWindowAttributes(hwnd, hud_chroma_key_colorref(), 0, LWA_COLORKEY);
            // Force the uniform-alpha path to re-run when we leave HUD mode.
            *applied_opacity = None;
            return;
        }

        // Normal: layered uniform-alpha opacity (style already applied above).
        let opacity_stale =
            !applied_opacity.is_some_and(|current| (current - opacity).abs() < f32::EPSILON);
        if (force_opacity || style_changed || opacity_stale)
            && SetLayeredWindowAttributes(hwnd, 0, (opacity * 255.0).round() as u8, LWA_ALPHA) != 0
        {
            *applied_opacity = Some(opacity);
        }
    }
}

pub(crate) fn apply_rounding_to_process_windows() {
    // SAFETY: EnumWindows invokes this callback with the documented ABI and valid HWND values.
    unsafe extern "system" fn apply_rounding(hwnd: HWND, process_id: LPARAM) -> i32 {
        let mut window_process_id = 0;
        // SAFETY: EnumWindows provides a valid top-level hwnd for this callback.
        unsafe {
            GetWindowThreadProcessId(hwnd, &mut window_process_id);
        }
        if window_process_id != process_id as u32 {
            return 1;
        }
        // SAFETY: hwnd belongs to this process and the attribute pointer is valid
        // for the duration of the synchronous DwmSetWindowAttribute call.
        unsafe {
            DwmSetWindowAttribute(
                hwnd,
                DWMWA_WINDOW_CORNER_PREFERENCE as u32,
                std::ptr::from_ref(&DWMWCP_ROUND).cast(),
                std::mem::size_of_val(&DWMWCP_ROUND) as u32,
            );
        }
        1
    }

    // SAFETY: The callback does not capture Rust references; lparam is only the
    // current process id cast through LPARAM for the duration of EnumWindows.
    unsafe {
        EnumWindows(Some(apply_rounding), std::process::id() as LPARAM);
    }
}

pub(crate) fn clear_process_windows_topmost(visible_only: bool) {
    // SAFETY: EnumWindows invokes this callback synchronously. The LPARAM points
    // to a stack request that remains valid for the duration of EnumWindows.
    unsafe extern "system" fn clear_topmost(hwnd: HWND, request: LPARAM) -> i32 {
        let request = unsafe { &*(request as *const TopmostWindowRequest) };
        let mut window_process_id = 0;
        // SAFETY: EnumWindows provides a valid top-level hwnd for this callback.
        unsafe {
            GetWindowThreadProcessId(hwnd, &mut window_process_id);
        }
        if window_process_id != request.process_id
            || (request.visible_only && unsafe { IsWindowVisible(hwnd) } == 0)
        {
            return 1;
        }
        set_window_topmost_raw(hwnd, false);
        1
    }

    let request = TopmostWindowRequest {
        process_id: std::process::id(),
        visible_only,
    };
    // SAFETY: The callback does not outlive request; EnumWindows is synchronous.
    unsafe {
        EnumWindows(
            Some(clear_topmost),
            std::ptr::from_ref(&request).addr() as LPARAM,
        );
    }
}

pub(crate) fn restore_visible_process_windows_topmost() {
    // SAFETY: EnumWindows invokes this callback synchronously. The LPARAM points
    // to a stack request that remains valid for the duration of EnumWindows.
    unsafe extern "system" fn restore_topmost(hwnd: HWND, request: LPARAM) -> i32 {
        let request = unsafe { &*(request as *const TopmostWindowRequest) };
        let mut window_process_id = 0;
        // SAFETY: EnumWindows provides a valid top-level hwnd for this callback.
        unsafe {
            GetWindowThreadProcessId(hwnd, &mut window_process_id);
        }
        if window_process_id != request.process_id || unsafe { IsWindowVisible(hwnd) } == 0 {
            return 1;
        }
        set_window_topmost_raw(hwnd, true);
        1
    }

    let request = TopmostWindowRequest {
        process_id: std::process::id(),
        visible_only: true,
    };
    // SAFETY: The callback does not outlive request; EnumWindows is synchronous.
    unsafe {
        EnumWindows(
            Some(restore_topmost),
            std::ptr::from_ref(&request).addr() as LPARAM,
        );
    }
}

pub(crate) fn set_window_topmost(hwnd: isize, topmost: bool) {
    set_window_topmost_raw(hwnd as HWND, topmost);
}

fn set_window_topmost_raw(hwnd: HWND, topmost: bool) {
    let insert_after = if topmost {
        HWND_TOPMOST
    } else {
        HWND_NOTOPMOST
    };
    // SAFETY: hwnd is either provided by eframe/raw-window-handle or by EnumWindows.
    // The call only changes Z-order and does not move, resize, or activate windows.
    unsafe {
        SetWindowPos(
            hwnd,
            insert_after,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
    }
}

struct TopmostWindowRequest {
    process_id: u32,
    visible_only: bool,
}
