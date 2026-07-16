use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::path::Path;
use windows_sys::Win32::Foundation::{HWND, LPARAM, POINT, RECT};
use windows_sys::Win32::Graphics::Dwm::{
    DWMNCRP_DISABLED, DWMNCRP_ENABLED, DWMWA_BORDER_COLOR, DWMWA_NCRENDERING_POLICY,
    DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmSetWindowAttribute,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GWL_EXSTYLE, GetCursorPos, GetWindowLongPtrW, GetWindowRect, GetWindowTextW,
    GetWindowThreadProcessId, IsWindowVisible, LWA_ALPHA, SetLayeredWindowAttributes,
    SetWindowLongPtrW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT,
};

/// DWM border-color sentinels (see `DWMWA_BORDER_COLOR`): suppress the 1px window
/// border for a true overlay, or restore the system default.
const DWMWA_COLOR_NONE: u32 = 0xFFFF_FFFE;
const DWMWA_COLOR_DEFAULT: u32 = 0xFFFF_FFFF;

pub(crate) fn open_directory(path: &Path) -> Result<(), String> {
    std::process::Command::new("explorer.exe")
        .arg(path)
        .spawn()
        .map(|_| ())
        .map_err(|error| error.to_string())
}

pub(crate) struct WindowAttributeConfig {
    pub(crate) opacity: f32,
    pub(crate) force_opacity: bool,
    pub(crate) hud_overlay: bool,
    pub(crate) passthrough: bool,
}

pub(crate) fn apply_window_attributes(
    frame: &eframe::Frame,
    config: WindowAttributeConfig,
    applied_opacity: &mut Option<f32>,
    corner_applied_hwnd: &mut Option<isize>,
) {
    let opacity = config.opacity.clamp(0.35, 1.0);
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

        // Normal windows use layered uniform alpha for the opacity slider. HUD
        // mode prefers the transparent framebuffer created by eframe/winit so
        // text/images stay opaque while empty pixels keep real alpha. Click-through
        // still needs a Win32 WS_EX_TRANSPARENT fallback; egui's viewport command
        // alone is not reliable on every Windows compositor path.
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let mut new_style = style;
        if config.hud_overlay {
            // Match egui_overlay/glfw's transparent-framebuffer model: the HUD
            // should only keep layered style when needed for reliable mouse
            // pass-through.
            if config.passthrough {
                new_style |= WS_EX_LAYERED as isize;
            } else {
                new_style &= !(WS_EX_LAYERED as isize);
            }
            if config.passthrough {
                new_style |= WS_EX_TRANSPARENT as isize;
            } else {
                new_style &= !(WS_EX_TRANSPARENT as isize);
            }
        } else {
            new_style |= WS_EX_LAYERED as isize;
            if config.passthrough {
                new_style |= WS_EX_TRANSPARENT as isize;
            } else {
                new_style &= !(WS_EX_TRANSPARENT as isize);
            }
        }
        let style_changed = new_style != style;
        if style_changed {
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, new_style);
        }

        // No window border in HUD mode so it reads as a true overlay, not a frame.
        let border = if config.hud_overlay {
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

        let nc_policy = if config.hud_overlay {
            DWMNCRP_DISABLED
        } else {
            DWMNCRP_ENABLED
        };
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_NCRENDERING_POLICY as u32,
            std::ptr::from_ref(&nc_policy).cast(),
            std::mem::size_of_val(&nc_policy) as u32,
        );

        if config.hud_overlay {
            // HUD transparency comes from the transparent swapchain/clear colour,
            // not layered uniform alpha. If click-through keeps WS_EX_LAYERED
            // enabled, reset any opacity slider alpha left from normal mode.
            if (new_style & WS_EX_LAYERED as isize) != 0 {
                let _ = SetLayeredWindowAttributes(hwnd, 0, 255, LWA_ALPHA);
            }
            // Force the uniform-alpha path to re-run when we leave HUD mode.
            *applied_opacity = None;
            return;
        }

        // Normal: layered uniform-alpha opacity (style already applied above).
        let opacity_stale =
            !applied_opacity.is_some_and(|current| (current - opacity).abs() < f32::EPSILON);
        if (config.force_opacity || style_changed || opacity_stale)
            && SetLayeredWindowAttributes(hwnd, 0, (opacity * 255.0).round() as u8, LWA_ALPHA) != 0
        {
            *applied_opacity = Some(opacity);
        }
    }
}

/// Global cursor position in physical desktop pixels. The notification island
/// polls this instead of egui pointer events because its window is
/// click-through (`WS_EX_TRANSPARENT`) whenever the cursor is *not* over the
/// capsule — in that state the window receives no mouse messages at all.
pub(crate) fn cursor_screen_pos() -> Option<(i32, i32)> {
    let mut point = POINT { x: 0, y: 0 };
    // SAFETY: GetCursorPos writes into a valid stack POINT.
    (unsafe { GetCursorPos(&mut point) } != 0).then_some((point.x, point.y))
}

/// Find this process's top-level window whose title matches `title` exactly.
/// eframe never exposes secondary-viewport HWNDs, so the island window is
/// located by the unique title its `ViewportBuilder` sets.
pub(crate) fn find_process_window_by_title(title: &str) -> Option<isize> {
    struct Search {
        process_id: u32,
        title: Vec<u16>,
        found: isize,
    }

    // SAFETY: EnumWindows invokes this callback with the documented ABI; lparam
    // points at the caller's stack `Search` for the duration of EnumWindows.
    unsafe extern "system" fn visit(hwnd: HWND, lparam: LPARAM) -> i32 {
        // SAFETY: lparam is the Search pointer passed below, valid for the call.
        let search = unsafe { &mut *(lparam as *mut Search) };
        let mut window_process_id = 0;
        // SAFETY: EnumWindows provides a valid top-level hwnd for this callback.
        unsafe {
            GetWindowThreadProcessId(hwnd, &mut window_process_id);
        }
        if window_process_id != search.process_id {
            return 1;
        }
        let mut buffer = [0_u16; 128];
        // SAFETY: buffer is a valid mutable slice for the given length.
        let length =
            unsafe { GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32) } as usize;
        if buffer[..length] == search.title[..] {
            search.found = hwnd as isize;
            return 0;
        }
        1
    }

    let mut search = Search {
        process_id: std::process::id(),
        title: title.encode_utf16().collect(),
        found: 0,
    };
    // SAFETY: the callback only dereferences lparam during this synchronous call.
    unsafe {
        EnumWindows(Some(visit), std::ptr::from_mut(&mut search) as LPARAM);
    }
    (search.found != 0).then_some(search.found)
}

/// One-time overlay styling for the notification island window: never steal
/// focus even when clicked (`WS_EX_NOACTIVATE` — the game must stay in the
/// foreground), stay out of Alt-Tab (`WS_EX_TOOLWINDOW`), and drop the DWM
/// border/non-client chrome like the HUD overlay does.
pub(crate) fn apply_island_base_style(hwnd: isize) {
    let hwnd = hwnd as HWND;
    // SAFETY: hwnd belongs to a live window of this process; the DWM attribute
    // pointers reference local constants for the duration of each call.
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let new_style = style | WS_EX_NOACTIVATE as isize | WS_EX_TOOLWINDOW as isize;
        if new_style != style {
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, new_style);
        }
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_BORDER_COLOR as u32,
            std::ptr::from_ref(&DWMWA_COLOR_NONE).cast(),
            std::mem::size_of_val(&DWMWA_COLOR_NONE) as u32,
        );
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_NCRENDERING_POLICY as u32,
            std::ptr::from_ref(&DWMNCRP_DISABLED).cast(),
            std::mem::size_of_val(&DWMNCRP_DISABLED) as u32,
        );
    }
}

/// Toggle click-through on the island window. Mirrors the HUD passthrough
/// path in [`apply_window_attributes`]: `WS_EX_TRANSPARENT` needs
/// `WS_EX_LAYERED` alongside it to reliably pass hit-testing through on every
/// compositor path, and a layered window must have its uniform alpha reset to
/// opaque so the transparent-framebuffer rendering stays untouched.
pub(crate) fn set_island_click_through(hwnd: isize, click_through: bool) {
    let hwnd = hwnd as HWND;
    // SAFETY: hwnd belongs to a live window of this process.
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let mut new_style = style;
        if click_through {
            new_style |= (WS_EX_LAYERED | WS_EX_TRANSPARENT) as isize;
        } else {
            new_style &= !((WS_EX_LAYERED | WS_EX_TRANSPARENT) as isize);
        }
        if new_style != style {
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, new_style);
            if (new_style & WS_EX_LAYERED as isize) != 0 {
                let _ = SetLayeredWindowAttributes(hwnd, 0, 255, LWA_ALPHA);
            }
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
        if !is_visible_content_window(hwnd) {
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

fn is_visible_content_window(hwnd: HWND) -> bool {
    // SAFETY: hwnd is supplied by EnumWindows. GetWindowRect writes into a valid
    // stack RECT, and IsWindowVisible only queries window state.
    unsafe {
        if IsWindowVisible(hwnd) == 0 {
            return false;
        }
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        if GetWindowRect(hwnd, &mut rect) == 0 {
            return false;
        }
        let width = rect.right - rect.left;
        let height = rect.bottom - rect.top;
        width >= 160 && height >= 120
    }
}

/// Owner handle for a native dialog (e.g. `rfd::FileDialog::set_parent`), built
/// from a cached root-window HWND so a background thread can parent the dialog
/// without touching Win32 window state itself. Owned windows always render above
/// their owner regardless of topmost/z-order, which is what keeps a dialog from
/// being hidden behind an always-on-top window — without needing the
/// `EnumWindows`/`SetWindowPos` dance that `clear_process_windows_topmost` and
/// friends use elsewhere (and which deadlocks on this app's wgpu backend if run
/// from the UI thread's own frame callback, or blocks the caller if run
/// cross-thread against a UI thread that never drains that message).
#[derive(Clone, Copy)]
pub(crate) struct DialogOwner(std::num::NonZeroIsize);

impl DialogOwner {
    pub(crate) fn from_hwnd(hwnd: Option<isize>) -> Option<Self> {
        std::num::NonZeroIsize::new(hwnd?).map(Self)
    }
}

impl raw_window_handle::HasWindowHandle for DialogOwner {
    fn window_handle(
        &self,
    ) -> Result<raw_window_handle::WindowHandle<'_>, raw_window_handle::HandleError> {
        let handle = raw_window_handle::Win32WindowHandle::new(self.0);
        // SAFETY: the wrapped HWND is only read (never dereferenced as a Rust
        // pointer) by rfd, which forwards it to `IFileOpenDialog::Show` as the
        // dialog's owner. No lifetime is actually borrowed here.
        Ok(unsafe {
            raw_window_handle::WindowHandle::borrow_raw(raw_window_handle::RawWindowHandle::Win32(
                handle,
            ))
        })
    }
}

impl raw_window_handle::HasDisplayHandle for DialogOwner {
    fn display_handle(
        &self,
    ) -> Result<raw_window_handle::DisplayHandle<'_>, raw_window_handle::HandleError> {
        Ok(raw_window_handle::DisplayHandle::windows())
    }
}
