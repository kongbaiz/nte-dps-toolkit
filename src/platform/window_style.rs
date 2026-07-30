//! Small Win32 window-shape helpers shared by desktop adapters.

use std::ffi::c_void;

use libloading::Library;
use windows_sys::Win32::Graphics::Dwm::{
    DWMWA_BORDER_COLOR, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmSetWindowAttribute,
};
use windows_sys::{Win32::Foundation::HWND, core::BOOL};

/// DWM sentinel that suppresses the system-drawn one-pixel window border.
const DWMWA_COLOR_NONE: u32 = 0xFFFF_FFFE;
const WCA_ACCENT_POLICY: u32 = 0x13;
const ACCENT_DISABLED: u32 = 0;
const ACCENT_ENABLE_ACRYLIC_BLUR_BEHIND: u32 = 4;
/// Acrylic rejects a fully transparent gradient. HTML supplies the visible tint.
const MINIMUM_ACRYLIC_ALPHA: u32 = 1 << 24;

#[repr(C)]
#[derive(Debug, PartialEq, Eq)]
struct AccentPolicy {
    accent_state: u32,
    accent_flags: u32,
    gradient_color: u32,
    animation_id: u32,
}

#[repr(C)]
struct WindowCompositionAttributeData {
    attribute: u32,
    data: *mut c_void,
    size: usize,
}

type SetWindowCompositionAttribute =
    unsafe extern "system" fn(HWND, *mut WindowCompositionAttributeData) -> BOOL;

fn acrylic_policy(enabled: bool) -> AccentPolicy {
    AccentPolicy {
        accent_state: if enabled {
            ACCENT_ENABLE_ACRYLIC_BLUR_BEHIND
        } else {
            ACCENT_DISABLED
        },
        accent_flags: 0,
        gradient_color: if enabled { MINIMUM_ACRYLIC_ALPHA } else { 0 },
        animation_id: 0,
    }
}

/// Apply the legacy composition Acrylic path on every supported Windows build.
///
/// Tauri's modern `Acrylic` maps to the transient system backdrop on recent
/// Windows 11 builds, which fades when an always-on-top HUD loses focus.
/// Tauri's `Blur` maps to an accent policy with a fully transparent gradient,
/// which WebView2 can present as black and can flash while the window moves.
/// The Acrylic accent policy remains active for inactive windows and keeps the
/// HTML layer responsible for the visible dark tint.
pub fn apply_persistent_acrylic(hwnd: isize, enabled: bool) -> Result<(), String> {
    // SAFETY: The system `user32.dll` is loaded only to resolve the documented
    // process-local function address. The library remains alive for the entire
    // symbol use below.
    let user32 = unsafe { Library::new("user32.dll") }
        .map_err(|error| format!("load user32.dll: {error}"))?;
    // SAFETY: The symbol signature matches SetWindowCompositionAttribute's
    // Win32 ABI. The policy and data pointers remain valid for the synchronous
    // call, and the HWND originates from the active Tauri window.
    let set_window_composition_attribute = unsafe {
        user32
            .get::<SetWindowCompositionAttribute>(b"SetWindowCompositionAttribute\0")
            .map_err(|error| format!("resolve SetWindowCompositionAttribute: {error}"))?
    };
    let mut policy = acrylic_policy(enabled);
    let mut data = WindowCompositionAttributeData {
        attribute: WCA_ACCENT_POLICY,
        data: std::ptr::from_mut(&mut policy).cast(),
        size: std::mem::size_of::<AccentPolicy>(),
    };
    // SAFETY: `data` points to a correctly sized `AccentPolicy` for the
    // duration of this synchronous Win32 call.
    let applied =
        unsafe { set_window_composition_attribute(hwnd as HWND, std::ptr::from_mut(&mut data)) };
    if applied == 0 {
        return Err(format!(
            "SetWindowCompositionAttribute rejected the Acrylic policy: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

/// Round the native HWND itself and suppress its system border.
///
/// CSS clipping only rounds WebView content; native Acrylic still occupies the
/// complete HWND. Applying both DWM attributes keeps the compositor surface and
/// the HTML surface on the same borderless rounded boundary.
pub fn apply_borderless_rounding(hwnd: isize) -> Result<(), i32> {
    let hwnd = hwnd as HWND;
    // SAFETY: `hwnd` is supplied by the active Tauri window. Both attribute
    // pointers reference stack values for the duration of the synchronous DWM
    // calls, and their byte sizes match the documented attribute types.
    unsafe {
        let corner_result = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE as u32,
            std::ptr::from_ref(&DWMWCP_ROUND).cast(),
            std::mem::size_of_val(&DWMWCP_ROUND) as u32,
        );
        if corner_result < 0 {
            return Err(corner_result);
        }

        let border_result = DwmSetWindowAttribute(
            hwnd,
            DWMWA_BORDER_COLOR as u32,
            std::ptr::from_ref(&DWMWA_COLOR_NONE).cast(),
            std::mem::size_of_val(&DWMWA_COLOR_NONE) as u32,
        );
        if border_result < 0 {
            return Err(border_result);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persistent_acrylic_uses_nonzero_alpha_without_adding_a_native_tint() {
        assert_eq!(
            acrylic_policy(true),
            AccentPolicy {
                accent_state: ACCENT_ENABLE_ACRYLIC_BLUR_BEHIND,
                accent_flags: 0,
                gradient_color: MINIMUM_ACRYLIC_ALPHA,
                animation_id: 0,
            }
        );
    }

    #[test]
    fn disabling_acrylic_clears_the_accent_policy() {
        assert_eq!(
            acrylic_policy(false),
            AccentPolicy {
                accent_state: ACCENT_DISABLED,
                accent_flags: 0,
                gradient_color: 0,
                animation_id: 0,
            }
        );
    }
}
