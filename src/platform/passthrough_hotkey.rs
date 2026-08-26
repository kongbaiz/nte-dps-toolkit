use std::{
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread,
    time::Duration,
};

use windows_sys::Win32::{
    Foundation::{GetLastError, LPARAM, LRESULT, WPARAM},
    UI::{
        Input::KeyboardAndMouse::{
            GetAsyncKeyState, VK_CONTROL, VK_DELETE, VK_DOWN, VK_END, VK_F1, VK_F2, VK_F3, VK_F4,
            VK_F5, VK_F6, VK_F7, VK_F8, VK_F9, VK_F10, VK_F11, VK_F12, VK_HOME, VK_INSERT, VK_LEFT,
            VK_MENU, VK_NEXT, VK_PRIOR, VK_RIGHT, VK_SHIFT, VK_SPACE, VK_UP,
        },
        WindowsAndMessaging::{
            CallNextHookEx, KBDLLHOOKSTRUCT, MSG, PM_REMOVE, PeekMessageW, SetWindowsHookExW,
            UnhookWindowsHookEx, WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
        },
    },
};

use crate::storage::config::{GlobalHotkeyAction, GlobalHotkeys, HotkeyBinding, HotkeyKey};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PassthroughHotkeyEvent {
    Toggle,
    GlobalAction(GlobalHotkeyAction),
    HookInstalled,
    HookInstallFailed { error: u32 },
}

#[derive(Default)]
struct HookState {
    sender: Option<Sender<PassthroughHotkeyEvent>>,
    instance_id: u64,
}

static HOOK_STATE: OnceLock<Mutex<HookState>> = OnceLock::new();
static HOOK_INSTANCE_COUNTER: AtomicU64 = AtomicU64::new(1);
static CONFIGURED_BINDING: AtomicU32 = AtomicU32::new(VK_HOME as u32);
static CONFIGURED_KEY_DOWN: AtomicBool = AtomicBool::new(false);
static GLOBAL_KEY_DOWN: AtomicU64 = AtomicU64::new(0);
static GLOBAL_HOTKEYS: OnceLock<Mutex<GlobalHotkeys>> = OnceLock::new();

fn reset_pressed_key_state() {
    CONFIGURED_KEY_DOWN.store(false, Ordering::Release);
    GLOBAL_KEY_DOWN.store(0, Ordering::Release);
}

fn disabled_global_hotkeys() -> GlobalHotkeys {
    GlobalHotkeys {
        enabled: false,
        capture: None,
        reset: None,
        hud: None,
        new_round: None,
    }
}

/// Callback routing is ephemeral OS state. Discard the complete value after
/// poison and report recovery so the triggering callback can be dropped.
fn lock_hotkey_state(state: &Mutex<HookState>) -> (std::sync::MutexGuard<'_, HookState>, bool) {
    match state.lock() {
        Ok(guard) => (guard, false),
        Err(poisoned) => {
            let mut guard = poisoned.into_inner();
            *guard = HookState::default();
            state.clear_poison();
            reset_pressed_key_state();
            (guard, true)
        }
    }
}

fn lock_or_reset_hotkey_state(state: &Mutex<HookState>) -> std::sync::MutexGuard<'_, HookState> {
    lock_hotkey_state(state).0
}

fn hook_sender_for_event(state: &Mutex<HookState>) -> Option<Sender<PassthroughHotkeyEvent>> {
    let (guard, recovered) = lock_hotkey_state(state);
    (!recovered).then(|| guard.sender.clone()).flatten()
}

/// User-configured bindings are not reconstructable from `Default`, because
/// the default enables actions. Poison installs an explicitly disabled set.
fn lock_global_hotkeys(
    hotkeys: &Mutex<GlobalHotkeys>,
) -> (std::sync::MutexGuard<'_, GlobalHotkeys>, bool) {
    match hotkeys.lock() {
        Ok(guard) => (guard, false),
        Err(poisoned) => {
            let mut guard = poisoned.into_inner();
            *guard = disabled_global_hotkeys();
            hotkeys.clear_poison();
            reset_pressed_key_state();
            (guard, true)
        }
    }
}

fn global_hotkeys_for_event(hotkeys: &Mutex<GlobalHotkeys>) -> Option<GlobalHotkeys> {
    let (guard, recovered) = lock_global_hotkeys(hotkeys);
    (!recovered).then_some(*guard)
}

fn hotkey_virtual_key(key: HotkeyKey) -> u32 {
    match key {
        HotkeyKey::A => 0x41,
        HotkeyKey::B => 0x42,
        HotkeyKey::C => 0x43,
        HotkeyKey::D => 0x44,
        HotkeyKey::E => 0x45,
        HotkeyKey::F => 0x46,
        HotkeyKey::G => 0x47,
        HotkeyKey::H => 0x48,
        HotkeyKey::I => 0x49,
        HotkeyKey::J => 0x4A,
        HotkeyKey::K => 0x4B,
        HotkeyKey::L => 0x4C,
        HotkeyKey::M => 0x4D,
        HotkeyKey::N => 0x4E,
        HotkeyKey::O => 0x4F,
        HotkeyKey::P => 0x50,
        HotkeyKey::Q => 0x51,
        HotkeyKey::R => 0x52,
        HotkeyKey::S => 0x53,
        HotkeyKey::T => 0x54,
        HotkeyKey::U => 0x55,
        HotkeyKey::V => 0x56,
        HotkeyKey::W => 0x57,
        HotkeyKey::X => 0x58,
        HotkeyKey::Y => 0x59,
        HotkeyKey::Z => 0x5A,
        HotkeyKey::Digit0 => 0x30,
        HotkeyKey::Digit1 => 0x31,
        HotkeyKey::Digit2 => 0x32,
        HotkeyKey::Digit3 => 0x33,
        HotkeyKey::Digit4 => 0x34,
        HotkeyKey::Digit5 => 0x35,
        HotkeyKey::Digit6 => 0x36,
        HotkeyKey::Digit7 => 0x37,
        HotkeyKey::Digit8 => 0x38,
        HotkeyKey::Digit9 => 0x39,
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
        HotkeyKey::Home => VK_HOME as u32,
        HotkeyKey::End => VK_END as u32,
        HotkeyKey::Insert => VK_INSERT as u32,
        HotkeyKey::Delete => VK_DELETE as u32,
        HotkeyKey::PageUp => VK_PRIOR as u32,
        HotkeyKey::PageDown => VK_NEXT as u32,
        HotkeyKey::ArrowUp => VK_UP as u32,
        HotkeyKey::ArrowDown => VK_DOWN as u32,
        HotkeyKey::ArrowLeft => VK_LEFT as u32,
        HotkeyKey::ArrowRight => VK_RIGHT as u32,
        HotkeyKey::Space => VK_SPACE as u32,
    }
}

fn global_key_bit(virtual_key: u32) -> Option<u64> {
    let index = match virtual_key {
        0x41..=0x5A => virtual_key - 0x41,
        0x30..=0x39 => 26 + virtual_key - 0x30,
        value if (VK_F1 as u32..=VK_F12 as u32).contains(&value) => 36 + value - VK_F1 as u32,
        value if value == VK_HOME as u32 => 48,
        value if value == VK_END as u32 => 49,
        value if value == VK_INSERT as u32 => 50,
        value if value == VK_DELETE as u32 => 51,
        value if value == VK_PRIOR as u32 => 52,
        value if value == VK_NEXT as u32 => 53,
        value if value == VK_UP as u32 => 54,
        value if value == VK_DOWN as u32 => 55,
        value if value == VK_LEFT as u32 => 56,
        value if value == VK_RIGHT as u32 => 57,
        value if value == VK_SPACE as u32 => 58,
        _ => return None,
    };
    Some(1_u64 << index)
}

fn async_key_down(virtual_key: u32) -> bool {
    // SAFETY: GetAsyncKeyState reads process-independent keyboard state for a valid virtual key.
    unsafe { GetAsyncKeyState(virtual_key as i32) < 0 }
}

fn binding_matches(binding: HotkeyBinding, virtual_key: u32) -> bool {
    hotkey_virtual_key(binding.key) == virtual_key
        && binding.ctrl == async_key_down(VK_CONTROL as u32)
        && binding.alt == async_key_down(VK_MENU as u32)
        && binding.shift == async_key_down(VK_SHIFT as u32)
}

const CTRL_BINDING_MASK: u32 = 1 << 16;
const ALT_BINDING_MASK: u32 = 1 << 17;
const SHIFT_BINDING_MASK: u32 = 1 << 18;
const VIRTUAL_KEY_MASK: u32 = 0xFFFF;

fn encode_binding(binding: HotkeyBinding) -> u32 {
    hotkey_virtual_key(binding.key)
        | if binding.ctrl { CTRL_BINDING_MASK } else { 0 }
        | if binding.alt { ALT_BINDING_MASK } else { 0 }
        | if binding.shift { SHIFT_BINDING_MASK } else { 0 }
}

fn encoded_binding_matches(encoded: u32, virtual_key: u32) -> bool {
    (encoded & VIRTUAL_KEY_MASK) == virtual_key
        && (encoded & CTRL_BINDING_MASK != 0) == async_key_down(VK_CONTROL as u32)
        && (encoded & ALT_BINDING_MASK != 0) == async_key_down(VK_MENU as u32)
        && (encoded & SHIFT_BINDING_MASK != 0) == async_key_down(VK_SHIFT as u32)
}

fn matching_global_action(virtual_key: u32) -> Option<GlobalHotkeyAction> {
    let hotkeys = global_hotkeys_for_event(
        GLOBAL_HOTKEYS.get_or_init(|| Mutex::new(GlobalHotkeys::default())),
    )?;
    hotkeys.enabled.then_some(())?;
    GlobalHotkeyAction::all().iter().copied().find(|action| {
        hotkeys
            .binding(*action)
            .is_some_and(|binding| binding_matches(binding, virtual_key))
    })
}

fn send_event(event: PassthroughHotkeyEvent) {
    let sender = HOOK_STATE.get().and_then(hook_sender_for_event);
    if let Some(sender) = sender {
        let _ = sender.send(event);
    }
}

// SAFETY: Windows calls this function with the WH_KEYBOARD_LL hook ABI and hook-owned
// parameters. The body forwards every event and only dereferences l_param for code >= 0.
unsafe extern "system" fn low_level_keyboard_proc(
    code: i32,
    w_param: WPARAM,
    l_param: LPARAM,
) -> LRESULT {
    if code >= 0 {
        // SAFETY: WH_KEYBOARD_LL provides a valid KBDLLHOOKSTRUCT for code >= 0.
        let keyboard = unsafe { &*(l_param as *const KBDLLHOOKSTRUCT) };
        let message = w_param as u32;
        let pressed = matches!(message, WM_KEYDOWN | WM_SYSKEYDOWN);
        let released = matches!(message, WM_KEYUP | WM_SYSKEYUP);
        let configured_binding = CONFIGURED_BINDING.load(Ordering::Acquire);
        let configured_key = configured_binding & VIRTUAL_KEY_MASK;

        if keyboard.vkCode == configured_key {
            if released {
                CONFIGURED_KEY_DOWN.store(false, Ordering::Release);
            } else if pressed {
                let initial_press = !CONFIGURED_KEY_DOWN.swap(true, Ordering::AcqRel);
                if initial_press && encoded_binding_matches(configured_binding, keyboard.vkCode) {
                    send_event(PassthroughHotkeyEvent::Toggle);
                }
            }
        }

        if let Some(bit) = global_key_bit(keyboard.vkCode) {
            if released {
                GLOBAL_KEY_DOWN.fetch_and(!bit, Ordering::AcqRel);
            } else if pressed {
                let initial_press = GLOBAL_KEY_DOWN.fetch_or(bit, Ordering::AcqRel) & bit == 0;
                if initial_press && let Some(action) = matching_global_action(keyboard.vkCode) {
                    send_event(PassthroughHotkeyEvent::GlobalAction(action));
                }
            }
        }
    }

    // SAFETY: Forwarding the hook parameters exactly as received is required by the API.
    unsafe { CallNextHookEx(std::ptr::null_mut(), code, w_param, l_param) }
}

pub struct PassthroughHotkeyHandle {
    instance_id: u64,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl PassthroughHotkeyHandle {
    pub fn start(
        hotkey: HotkeyBinding,
        global_hotkeys: GlobalHotkeys,
    ) -> Result<(Self, Receiver<PassthroughHotkeyEvent>), String> {
        let instance_id = HOOK_INSTANCE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = mpsc::channel();
        CONFIGURED_BINDING.store(encode_binding(hotkey), Ordering::Release);
        CONFIGURED_KEY_DOWN.store(false, Ordering::Release);
        GLOBAL_KEY_DOWN.store(0, Ordering::Release);
        Self::set_configuration(hotkey, global_hotkeys);
        {
            let state = HOOK_STATE.get_or_init(|| Mutex::new(HookState::default()));
            let mut state = lock_or_reset_hotkey_state(state);
            state.sender = Some(sender);
            state.instance_id = instance_id;
        }

        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let worker = thread::Builder::new()
            .name("hud-passthrough-hotkey".to_owned())
            .spawn(move || {
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
                    send_event(PassthroughHotkeyEvent::HookInstallFailed { error });
                    return;
                }
                send_event(PassthroughHotkeyEvent::HookInstalled);

                // SAFETY: MSG is a plain old data Windows structure and zero is a valid initial state.
                let mut message = unsafe { std::mem::zeroed::<MSG>() };
                while !worker_stop.load(Ordering::Acquire) {
                    // SAFETY: message points to valid storage; PM_REMOVE drains this thread's queue.
                    while unsafe {
                        PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_REMOVE)
                    } != 0
                    {}
                    thread::sleep(Duration::from_millis(8));
                }

                // SAFETY: hook was returned by SetWindowsHookExW on this thread and is released once.
                unsafe {
                    UnhookWindowsHookEx(hook);
                }
            })
            .map_err(|error| {
                clear_sender(instance_id);
                error.to_string()
            })?;

        Ok((
            Self {
                instance_id,
                stop,
                thread: Some(worker),
            },
            receiver,
        ))
    }

    pub fn set_configuration(hotkey: HotkeyBinding, global_hotkeys: GlobalHotkeys) {
        CONFIGURED_BINDING.store(encode_binding(hotkey), Ordering::Release);
        let (mut configured, _) = lock_global_hotkeys(
            GLOBAL_HOTKEYS.get_or_init(|| Mutex::new(GlobalHotkeys::default())),
        );
        *configured = global_hotkeys.sanitized();
        reset_pressed_key_state();
    }
}

impl Drop for PassthroughHotkeyHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.thread.take() {
            let _ = worker.join();
        }
        CONFIGURED_KEY_DOWN.store(false, Ordering::Release);
        GLOBAL_KEY_DOWN.store(0, Ordering::Release);
        clear_sender(self.instance_id);
    }
}

fn clear_sender(instance_id: u64) {
    let Some(state) = HOOK_STATE.get() else {
        return;
    };
    let mut state = lock_or_reset_hotkey_state(state);
    if state.instance_id == instance_id {
        state.sender = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hotkey_keys_map_to_letters_navigation_and_function_keys() {
        assert_eq!(hotkey_virtual_key(HotkeyKey::A), 0x41);
        assert_eq!(hotkey_virtual_key(HotkeyKey::Digit9), 0x39);
        assert_eq!(hotkey_virtual_key(HotkeyKey::Home), VK_HOME as u32);
        assert_eq!(hotkey_virtual_key(HotkeyKey::F9), VK_F9 as u32);
    }

    #[test]
    fn global_hotkey_keys_map_to_the_windows_function_key_range() {
        assert_eq!(hotkey_virtual_key(HotkeyKey::F1), VK_F1 as u32);
        assert_eq!(hotkey_virtual_key(HotkeyKey::F12), VK_F12 as u32);
        assert_eq!(global_key_bit(VK_F1 as u32), Some(1_u64 << 36));
        assert_eq!(global_key_bit(VK_F12 as u32), Some(1_u64 << 47));
    }

    #[test]
    fn passthrough_binding_encoding_preserves_key_and_modifiers() {
        let encoded = encode_binding(HotkeyBinding::new(true, false, true, HotkeyKey::Insert));
        assert_eq!(encoded & VIRTUAL_KEY_MASK, VK_INSERT as u32);
        assert_ne!(encoded & CTRL_BINDING_MASK, 0);
        assert_eq!(encoded & ALT_BINDING_MASK, 0);
        assert_ne!(encoded & SHIFT_BINDING_MASK, 0);
    }

    #[test]
    fn poisoned_hotkey_state_is_reset_before_reuse() {
        let state = Mutex::new(HookState {
            sender: None,
            instance_id: 41,
        });
        let _ = std::panic::catch_unwind(|| {
            let mut guard = state.lock().expect("test hook state");
            guard.instance_id = 99;
            panic!("poison test hook state");
        });

        let guard = lock_or_reset_hotkey_state(&state);
        assert_eq!(guard.instance_id, 0);
        assert!(guard.sender.is_none());
        drop(guard);
        assert!(!state.is_poisoned());
    }

    #[test]
    fn poisoned_global_hotkeys_disable_bindings_and_drop_triggering_event() {
        let hotkeys = Mutex::new(GlobalHotkeys::default());
        let _ = std::panic::catch_unwind(|| {
            let mut guard = hotkeys.lock().expect("test global hotkeys");
            guard.enabled = true;
            panic!("poison test global hotkeys");
        });

        assert!(global_hotkeys_for_event(&hotkeys).is_none());
        let recovered = global_hotkeys_for_event(&hotkeys).expect("recovered disabled snapshot");
        assert!(!recovered.enabled);
        assert!(recovered.capture.is_none());
        assert!(recovered.reset.is_none());
        assert!(recovered.hud.is_none());
        assert!(recovered.new_round.is_none());
        assert!(!hotkeys.is_poisoned());
    }

    #[test]
    fn poisoned_hook_state_drops_triggering_event_and_sender() {
        let (sender, _receiver) = mpsc::channel();
        let state = Mutex::new(HookState {
            sender: Some(sender),
            instance_id: 7,
        });
        let _ = std::panic::catch_unwind(|| {
            let mut guard = state.lock().expect("test hook state");
            guard.instance_id = 8;
            panic!("poison test hook sender");
        });

        assert!(hook_sender_for_event(&state).is_none());
        let recovered = lock_or_reset_hotkey_state(&state);
        assert!(recovered.sender.is_none());
        assert_eq!(recovered.instance_id, 0);
        assert!(!state.is_poisoned());
    }
}
