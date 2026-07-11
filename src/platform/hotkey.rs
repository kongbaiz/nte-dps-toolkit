use std::sync::{
    Arc, Mutex, OnceLock, RwLock,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::thread;
use std::time::Duration;

use crate::storage::config::{
    GlobalHotkeyAction, GlobalHotkeys, HotkeyBinding, HotkeyKey, PassthroughHotkey,
};
use crossbeam_channel::{Receiver, Sender, unbounded};
use eframe::egui;
use windows_sys::Win32::Foundation::{GetLastError, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    VK_CONTROL, VK_F1, VK_F2, VK_F3, VK_F4, VK_F5, VK_F6, VK_F7, VK_F8, VK_F9, VK_F10, VK_F11,
    VK_F12, VK_HOME, VK_INSERT, VK_K, VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_MENU, VK_RCONTROL,
    VK_RMENU, VK_RSHIFT, VK_SHIFT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetForegroundWindow, GetWindowThreadProcessId, KBDLLHOOKSTRUCT, MSG, PM_REMOVE,
    PeekMessageW, SetWindowsHookExW, UnhookWindowsHookEx, WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP,
    WM_SYSKEYDOWN, WM_SYSKEYUP,
};

#[derive(Default)]
struct HotkeyState {
    sender: Option<Sender<HotkeyEvent>>,
    context: Option<egui::Context>,
    instance_id: u64,
}

static HOTKEY_STATE: OnceLock<Mutex<HotkeyState>> = OnceLock::new();
static GLOBAL_HOTKEYS: OnceLock<RwLock<GlobalHotkeys>> = OnceLock::new();
static HOTKEY_INSTANCE_COUNTER: AtomicU64 = AtomicU64::new(1);
static PASSTHROUGH_VK: AtomicU64 = AtomicU64::new(VK_HOME as u64);
static MODIFIERS_DOWN: AtomicU64 = AtomicU64::new(0);
static WATCHED_KEYS_DOWN: AtomicU64 = AtomicU64::new(0);
static HOTKEY_RECORDING: AtomicBool = AtomicBool::new(false);

const MODIFIER_LEFT_CTRL: u64 = 1 << 0;
const MODIFIER_RIGHT_CTRL: u64 = 1 << 1;
const MODIFIER_GENERIC_CTRL: u64 = 1 << 2;
const MODIFIER_LEFT_ALT: u64 = 1 << 3;
const MODIFIER_RIGHT_ALT: u64 = 1 << 4;
const MODIFIER_GENERIC_ALT: u64 = 1 << 5;
const MODIFIER_LEFT_SHIFT: u64 = 1 << 6;
const MODIFIER_RIGHT_SHIFT: u64 = 1 << 7;
const MODIFIER_GENERIC_SHIFT: u64 = 1 << 8;
const MODIFIER_CTRL_MASK: u64 = MODIFIER_LEFT_CTRL | MODIFIER_RIGHT_CTRL | MODIFIER_GENERIC_CTRL;
const MODIFIER_ALT_MASK: u64 = MODIFIER_LEFT_ALT | MODIFIER_RIGHT_ALT | MODIFIER_GENERIC_ALT;
const MODIFIER_SHIFT_MASK: u64 =
    MODIFIER_LEFT_SHIFT | MODIFIER_RIGHT_SHIFT | MODIFIER_GENERIC_SHIFT;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HotkeyEvent {
    TogglePassthrough,
    GlobalAction(GlobalHotkeyAction),
    ToggleCommandPalette,
    ToggleDebug,
    HookInstalled,
    HookInstallFailed { error: u32 },
}

fn send_hotkey(event: HotkeyEvent) {
    let (sender, context) = HOTKEY_STATE
        .get()
        .map(|state| match state.lock() {
            Ok(state) => (state.sender.clone(), state.context.clone()),
            Err(poisoned) => {
                let state = poisoned.into_inner();
                (state.sender.clone(), state.context.clone())
            }
        })
        .unwrap_or((None, None));
    if let Some(sender) = sender {
        let _ = sender.send(event);
    }
    if let Some(context) = context {
        context.request_repaint();
    }
}

fn passthrough_virtual_key(hotkey: PassthroughHotkey) -> u64 {
    match hotkey {
        PassthroughHotkey::Home => VK_HOME as u64,
        PassthroughHotkey::Insert => VK_INSERT as u64,
        PassthroughHotkey::F8 => VK_F8 as u64,
        PassthroughHotkey::F9 => VK_F9 as u64,
    }
}

fn hotkey_virtual_key(key: HotkeyKey) -> u32 {
    match key {
        HotkeyKey::F1 => VK_F1 as u32,
        HotkeyKey::F2 => VK_F2 as u32,
        HotkeyKey::F3 => VK_F3 as u32,
        HotkeyKey::F4 => VK_F4 as u32,
        HotkeyKey::F5 => VK_F5 as u32,
        HotkeyKey::F6 => VK_F6 as u32,
        HotkeyKey::F7 => VK_F7 as u32,
        HotkeyKey::F8 => VK_F8 as u32,
        HotkeyKey::F9 => VK_F9 as u32,
        HotkeyKey::F10 => VK_F10 as u32,
        HotkeyKey::F11 => VK_F11 as u32,
        HotkeyKey::F12 => VK_F12 as u32,
    }
}

pub(crate) fn hotkey_key_to_egui(key: HotkeyKey) -> egui::Key {
    match key {
        HotkeyKey::F1 => egui::Key::F1,
        HotkeyKey::F2 => egui::Key::F2,
        HotkeyKey::F3 => egui::Key::F3,
        HotkeyKey::F4 => egui::Key::F4,
        HotkeyKey::F5 => egui::Key::F5,
        HotkeyKey::F6 => egui::Key::F6,
        HotkeyKey::F7 => egui::Key::F7,
        HotkeyKey::F8 => egui::Key::F8,
        HotkeyKey::F9 => egui::Key::F9,
        HotkeyKey::F10 => egui::Key::F10,
        HotkeyKey::F11 => egui::Key::F11,
        HotkeyKey::F12 => egui::Key::F12,
    }
}

pub(crate) fn passthrough_hotkey_to_egui(hotkey: PassthroughHotkey) -> egui::Key {
    match hotkey {
        PassthroughHotkey::Home => egui::Key::Home,
        PassthroughHotkey::Insert => egui::Key::Insert,
        PassthroughHotkey::F8 => egui::Key::F8,
        PassthroughHotkey::F9 => egui::Key::F9,
    }
}

pub(crate) fn hotkey_binding_matches_egui(
    binding: HotkeyBinding,
    modifiers: egui::Modifiers,
    key: egui::Key,
) -> bool {
    hotkey_key_to_egui(binding.key) == key
        && binding.ctrl == modifiers.ctrl
        && binding.alt == modifiers.alt
        && binding.shift == modifiers.shift
}

pub(crate) fn passthrough_hotkey_matches_egui(
    hotkey: PassthroughHotkey,
    modifiers: egui::Modifiers,
    key: egui::Key,
) -> bool {
    passthrough_hotkey_to_egui(hotkey) == key
        && !modifiers.ctrl
        && !modifiers.alt
        && !modifiers.shift
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct PressedModifiers {
    ctrl: bool,
    alt: bool,
    shift: bool,
}

fn current_global_hotkeys() -> GlobalHotkeys {
    let hotkeys = GLOBAL_HOTKEYS.get_or_init(|| RwLock::new(GlobalHotkeys::default()));
    match hotkeys.read() {
        Ok(hotkeys) => *hotkeys,
        Err(poisoned) => *poisoned.into_inner(),
    }
}

fn store_global_hotkeys(hotkeys: GlobalHotkeys) {
    let configured = GLOBAL_HOTKEYS.get_or_init(|| RwLock::new(GlobalHotkeys::default()));
    let mut configured = match configured.write() {
        Ok(configured) => configured,
        Err(poisoned) => poisoned.into_inner(),
    };
    *configured = hotkeys.sanitized();
}

fn modifier_bit(virtual_key: u32) -> Option<u64> {
    match virtual_key {
        key if key == VK_LCONTROL as u32 => Some(MODIFIER_LEFT_CTRL),
        key if key == VK_RCONTROL as u32 => Some(MODIFIER_RIGHT_CTRL),
        key if key == VK_CONTROL as u32 => Some(MODIFIER_GENERIC_CTRL),
        key if key == VK_LMENU as u32 => Some(MODIFIER_LEFT_ALT),
        key if key == VK_RMENU as u32 => Some(MODIFIER_RIGHT_ALT),
        key if key == VK_MENU as u32 => Some(MODIFIER_GENERIC_ALT),
        key if key == VK_LSHIFT as u32 => Some(MODIFIER_LEFT_SHIFT),
        key if key == VK_RSHIFT as u32 => Some(MODIFIER_RIGHT_SHIFT),
        key if key == VK_SHIFT as u32 => Some(MODIFIER_GENERIC_SHIFT),
        _ => None,
    }
}

fn update_modifier_state(virtual_key: u32, pressed: bool) -> bool {
    let Some(bit) = modifier_bit(virtual_key) else {
        return false;
    };
    if pressed {
        MODIFIERS_DOWN.fetch_or(bit, Ordering::Relaxed);
    } else {
        MODIFIERS_DOWN.fetch_and(!bit, Ordering::Relaxed);
    }
    true
}

fn pressed_modifiers() -> PressedModifiers {
    let bits = MODIFIERS_DOWN.load(Ordering::Relaxed);
    PressedModifiers {
        ctrl: bits & MODIFIER_CTRL_MASK != 0,
        alt: bits & MODIFIER_ALT_MASK != 0,
        shift: bits & MODIFIER_SHIFT_MASK != 0,
    }
}

fn binding_matches(binding: HotkeyBinding, virtual_key: u32, modifiers: PressedModifiers) -> bool {
    hotkey_virtual_key(binding.key) == virtual_key
        && binding.ctrl == modifiers.ctrl
        && binding.alt == modifiers.alt
        && binding.shift == modifiers.shift
}

fn configured_action_for_key(
    hotkeys: GlobalHotkeys,
    virtual_key: u32,
    modifiers: PressedModifiers,
) -> Option<GlobalHotkeyAction> {
    if !hotkeys.enabled {
        return None;
    }
    GlobalHotkeyAction::all().iter().copied().find(|action| {
        hotkeys
            .binding(*action)
            .is_some_and(|binding| binding_matches(binding, virtual_key, modifiers))
    })
}

fn is_configurable_key(virtual_key: u32) -> bool {
    (VK_F1 as u32..=VK_F12 as u32).contains(&virtual_key)
}

fn passthrough_matches(virtual_key: u32, modifiers: PressedModifiers) -> bool {
    unmodified_key_matches(
        PASSTHROUGH_VK.load(Ordering::Relaxed) as u32,
        virtual_key,
        modifiers,
    )
}

fn unmodified_key_matches(
    configured_key: u32,
    virtual_key: u32,
    modifiers: PressedModifiers,
) -> bool {
    configured_key == virtual_key && modifiers == PressedModifiers::default()
}

fn command_palette_matches(virtual_key: u32, modifiers: PressedModifiers) -> bool {
    virtual_key == VK_K as u32
        && modifiers
            == PressedModifiers {
                ctrl: true,
                alt: false,
                shift: false,
            }
}

fn watched_key_bit(virtual_key: u32) -> Option<u64> {
    if is_configurable_key(virtual_key) {
        return Some(1 << (virtual_key - VK_F1 as u32));
    }
    match virtual_key {
        key if key == VK_HOME as u32 => Some(1 << 12),
        key if key == VK_INSERT as u32 => Some(1 << 13),
        key if key == VK_K as u32 => Some(1 << 14),
        _ => None,
    }
}

fn claim_key_bit(state: &AtomicU64, bit: u64) -> bool {
    state.fetch_or(bit, Ordering::Relaxed) & bit == 0
}

fn first_keydown(virtual_key: u32) -> bool {
    watched_key_bit(virtual_key).is_none_or(|bit| claim_key_bit(&WATCHED_KEYS_DOWN, bit))
}

fn release_key(virtual_key: u32) {
    if let Some(bit) = watched_key_bit(virtual_key) {
        WATCHED_KEYS_DOWN.fetch_and(!bit, Ordering::Relaxed);
    }
}

fn reset_hook_state() {
    MODIFIERS_DOWN.store(0, Ordering::Relaxed);
    WATCHED_KEYS_DOWN.store(0, Ordering::Relaxed);
}

// SAFETY: Windows calls this function with the WH_KEYBOARD_LL hook ABI and hook-owned
// parameters. The body forwards unhandled events and only dereferences l_param for code >= 0.
unsafe extern "system" fn low_level_keyboard_proc(
    code: i32,
    w_param: WPARAM,
    l_param: LPARAM,
) -> LRESULT {
    if code >= 0 {
        // SAFETY: For WH_KEYBOARD_LL with code >= 0, l_param points to a KBDLLHOOKSTRUCT
        // that is valid for the duration of this callback.
        let keyboard = unsafe { &*(l_param as *const KBDLLHOOKSTRUCT) };
        let message = w_param as u32;
        let pressed = matches!(message, WM_KEYDOWN | WM_SYSKEYDOWN);
        let released = matches!(message, WM_KEYUP | WM_SYSKEYUP);
        if !pressed && !released {
            // SAFETY: Forwarding the hook parameters exactly as received is required by the API.
            return unsafe { CallNextHookEx(std::ptr::null_mut(), code, w_param, l_param) };
        }
        let virtual_key = keyboard.vkCode;
        let is_modifier = update_modifier_state(virtual_key, pressed);
        let initial_press = if pressed {
            first_keydown(virtual_key)
        } else {
            // Release by key rather than by the current modifiers: users may release
            // Ctrl/Alt/Shift before the action key, and the next press must not remain latched.
            release_key(virtual_key);
            false
        };

        // SAFETY: The low-level keyboard hook is called on a Windows hook thread; querying
        // the current foreground window does not require ownership of the returned HWND.
        let foreground = unsafe { GetForegroundWindow() };
        let mut foreground_process_id = 0_u32;
        if !foreground.is_null() {
            // SAFETY: foreground was returned by GetForegroundWindow and the out pointer is valid.
            unsafe {
                GetWindowThreadProcessId(foreground, &mut foreground_process_id);
            }
        }
        // Foreground app input is consumed through egui so shortcut recording and text focus stay
        // authoritative. Modifier/latch bookkeeping above still runs to avoid stale global state.
        if foreground_process_id == std::process::id() || is_modifier || !pressed || !initial_press
        {
            // SAFETY: Forwarding the hook parameters exactly as received is required by the API.
            return unsafe { CallNextHookEx(std::ptr::null_mut(), code, w_param, l_param) };
        }

        let modifiers = pressed_modifiers();
        if HOTKEY_RECORDING.load(Ordering::Relaxed) {
            // Shortcut capture only accepts input from the focused Console viewport. If focus
            // changes before the UI observes it, suppress configured actions rather than firing
            // the old binding in the game or another application.
            return unsafe { CallNextHookEx(std::ptr::null_mut(), code, w_param, l_param) };
        }
        if passthrough_matches(virtual_key, modifiers) {
            send_hotkey(HotkeyEvent::TogglePassthrough);
        }
        if is_configurable_key(virtual_key)
            && let Some(action) =
                configured_action_for_key(current_global_hotkeys(), virtual_key, modifiers)
        {
            send_hotkey(HotkeyEvent::GlobalAction(action));
        }
        if command_palette_matches(virtual_key, modifiers) {
            send_hotkey(HotkeyEvent::ToggleCommandPalette);
        }
        if virtual_key == VK_F12 as u32 && modifiers == PressedModifiers::default() {
            send_hotkey(HotkeyEvent::ToggleDebug);
        }
    }
    // SAFETY: Forwarding the hook parameters exactly as received is required by the API.
    unsafe { CallNextHookEx(std::ptr::null_mut(), code, w_param, l_param) }
}

pub struct HotkeyHandle {
    instance_id: u64,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl HotkeyHandle {
    pub fn start(
        context: egui::Context,
        passthrough_hotkey: PassthroughHotkey,
        global_hotkeys: GlobalHotkeys,
    ) -> (Self, Receiver<HotkeyEvent>) {
        let instance_id = HOTKEY_INSTANCE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = unbounded();
        PASSTHROUGH_VK.store(
            passthrough_virtual_key(passthrough_hotkey),
            Ordering::Relaxed,
        );
        store_global_hotkeys(global_hotkeys);
        HOTKEY_RECORDING.store(false, Ordering::Relaxed);
        reset_hook_state();
        {
            let state = HOTKEY_STATE.get_or_init(|| Mutex::new(HotkeyState::default()));
            let mut state = match state.lock() {
                Ok(state) => state,
                Err(poisoned) => poisoned.into_inner(),
            };
            state.sender = Some(sender);
            state.context = Some(context);
            state.instance_id = instance_id;
        }
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let thread = thread::spawn(move || {
            // SAFETY: Installs a process-local low-level keyboard hook with a static callback.
            let hook = unsafe {
                SetWindowsHookExW(
                    WH_KEYBOARD_LL,
                    Some(low_level_keyboard_proc),
                    std::ptr::null_mut(),
                    0,
                )
            };
            if hook.is_null() {
                // SAFETY: GetLastError reads the calling thread's last Windows error code.
                let error = unsafe { GetLastError() };
                send_hotkey(HotkeyEvent::HookInstallFailed { error });
                return;
            }
            send_hotkey(HotkeyEvent::HookInstalled);

            // SAFETY: MSG is a plain old data Win32 struct and zero is a valid initial state.
            let mut message = unsafe { std::mem::zeroed::<MSG>() };
            while !worker_stop.load(Ordering::Relaxed) {
                // SAFETY: message points to valid storage and PM_REMOVE drains this thread's queue.
                while unsafe { PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_REMOVE) }
                    != 0
                {}
                thread::sleep(Duration::from_millis(8));
            }

            // SAFETY: hook was returned by SetWindowsHookExW in this thread and is unhooked once.
            unsafe {
                UnhookWindowsHookEx(hook);
            }
        });

        (
            Self {
                instance_id,
                stop,
                thread: Some(thread),
            },
            receiver,
        )
    }

    pub fn set_passthrough_hotkey(&self, hotkey: PassthroughHotkey) {
        PASSTHROUGH_VK.store(passthrough_virtual_key(hotkey), Ordering::Relaxed);
    }

    pub fn set_global_hotkeys(&self, hotkeys: GlobalHotkeys) {
        store_global_hotkeys(hotkeys);
    }

    pub fn set_recording(&self, recording: bool) {
        HOTKEY_RECORDING.store(recording, Ordering::Relaxed);
    }
}

impl Drop for HotkeyHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        HOTKEY_RECORDING.store(false, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        reset_hook_state();
        let Some(state) = HOTKEY_STATE.get() else {
            return;
        };
        let mut state = match state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        };
        if state.instance_id == self.instance_id {
            state.sender = None;
            state.context = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configured_actions_require_exact_modifiers() {
        let hotkeys = GlobalHotkeys::default();
        let ctrl = PressedModifiers {
            ctrl: true,
            ..Default::default()
        };
        let ctrl_shift = PressedModifiers {
            ctrl: true,
            shift: true,
            ..Default::default()
        };

        assert_eq!(
            configured_action_for_key(hotkeys, VK_F9 as u32, ctrl),
            Some(GlobalHotkeyAction::ToggleCapture)
        );
        assert_eq!(
            configured_action_for_key(hotkeys, VK_F10 as u32, ctrl),
            Some(GlobalHotkeyAction::ResetSession)
        );
        assert_eq!(
            configured_action_for_key(hotkeys, VK_F11 as u32, ctrl),
            Some(GlobalHotkeyAction::ToggleHud)
        );
        assert_eq!(
            configured_action_for_key(hotkeys, VK_F9 as u32, PressedModifiers::default()),
            None
        );
        assert_eq!(
            configured_action_for_key(hotkeys, VK_F9 as u32, ctrl_shift),
            None
        );
        assert_eq!(
            configured_action_for_key(
                GlobalHotkeys {
                    enabled: false,
                    ..hotkeys
                },
                VK_F9 as u32,
                ctrl,
            ),
            None
        );
    }

    #[test]
    fn ctrl_f9_does_not_match_plain_f9_passthrough() {
        let ctrl = PressedModifiers {
            ctrl: true,
            ..Default::default()
        };

        assert!(unmodified_key_matches(
            VK_F9 as u32,
            VK_F9 as u32,
            PressedModifiers::default(),
        ));
        assert!(!unmodified_key_matches(VK_F9 as u32, VK_F9 as u32, ctrl,));
    }

    #[test]
    fn command_palette_is_fixed_to_ctrl_k() {
        let ctrl = PressedModifiers {
            ctrl: true,
            ..Default::default()
        };
        let ctrl_alt = PressedModifiers {
            ctrl: true,
            alt: true,
            ..Default::default()
        };

        assert!(command_palette_matches(VK_K as u32, ctrl));
        assert!(!command_palette_matches(VK_K as u32, ctrl_alt));
        assert!(!command_palette_matches(VK_F9 as u32, ctrl));
    }

    #[test]
    fn latch_suppresses_repeat_until_keyup() {
        let state = AtomicU64::new(0);
        let bit = watched_key_bit(VK_F9 as u32).expect("F9 should be watched");

        assert!(claim_key_bit(&state, bit));
        assert!(!claim_key_bit(&state, bit));
        state.fetch_and(!bit, Ordering::Relaxed);
        assert!(claim_key_bit(&state, bit));
    }

    #[test]
    fn every_config_key_has_a_distinct_function_key_code() {
        let keys = HotkeyKey::all()
            .iter()
            .map(|key| hotkey_virtual_key(*key))
            .collect::<Vec<_>>();
        let mut unique = keys.clone();
        unique.sort_unstable();
        unique.dedup();

        assert_eq!(unique.len(), keys.len());
        assert_eq!(hotkey_virtual_key(HotkeyKey::F1), VK_F1 as u32);
        assert_eq!(hotkey_virtual_key(HotkeyKey::F12), VK_F12 as u32);
    }

    #[test]
    fn local_hotkeys_match_exact_egui_modifiers() {
        let binding = HotkeyBinding::new(true, false, false, HotkeyKey::F9);
        let ctrl = egui::Modifiers {
            ctrl: true,
            ..Default::default()
        };
        let ctrl_shift = egui::Modifiers {
            ctrl: true,
            shift: true,
            ..Default::default()
        };

        assert!(hotkey_binding_matches_egui(binding, ctrl, egui::Key::F9));
        assert!(!hotkey_binding_matches_egui(
            binding,
            ctrl_shift,
            egui::Key::F9
        ));
        assert!(!hotkey_binding_matches_egui(binding, ctrl, egui::Key::F10));
        assert!(!hotkey_binding_matches_egui(
            HotkeyBinding::new(false, false, false, HotkeyKey::F9),
            ctrl,
            egui::Key::F9
        ));
        assert!(passthrough_hotkey_matches_egui(
            PassthroughHotkey::F9,
            egui::Modifiers::default(),
            egui::Key::F9
        ));
        assert!(!passthrough_hotkey_matches_egui(
            PassthroughHotkey::F9,
            ctrl,
            egui::Key::F9
        ));
    }
}
