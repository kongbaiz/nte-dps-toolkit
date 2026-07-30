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
            GetAsyncKeyState, VK_CONTROL, VK_F8, VK_F9, VK_HOME, VK_INSERT, VK_MENU, VK_SHIFT,
        },
        WindowsAndMessaging::{
            CallNextHookEx, KBDLLHOOKSTRUCT, MSG, PM_REMOVE, PeekMessageW, SetWindowsHookExW,
            UnhookWindowsHookEx, WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
        },
    },
};

use crate::storage::config::PassthroughHotkey;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PassthroughHotkeyEvent {
    Toggle,
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
static CONFIGURED_KEY: AtomicU32 = AtomicU32::new(VK_HOME as u32);
static CONFIGURED_KEY_DOWN: AtomicBool = AtomicBool::new(false);

fn virtual_key(hotkey: PassthroughHotkey) -> u32 {
    match hotkey {
        PassthroughHotkey::Home => VK_HOME as u32,
        PassthroughHotkey::Insert => VK_INSERT as u32,
        PassthroughHotkey::F8 => VK_F8 as u32,
        PassthroughHotkey::F9 => VK_F9 as u32,
    }
}

fn async_key_down(virtual_key: u32) -> bool {
    // SAFETY: GetAsyncKeyState reads process-independent keyboard state for a valid virtual key.
    unsafe { GetAsyncKeyState(virtual_key as i32) < 0 }
}

fn modifiers_down() -> bool {
    async_key_down(VK_CONTROL as u32)
        || async_key_down(VK_MENU as u32)
        || async_key_down(VK_SHIFT as u32)
}

fn matches_unmodified_press(configured_key: u32, virtual_key: u32, has_modifiers: bool) -> bool {
    configured_key == virtual_key && !has_modifiers
}

fn send_event(event: PassthroughHotkeyEvent) {
    let sender = HOOK_STATE.get().and_then(|state| match state.lock() {
        Ok(state) => state.sender.clone(),
        Err(poisoned) => poisoned.into_inner().sender.clone(),
    });
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
        let configured_key = CONFIGURED_KEY.load(Ordering::Acquire);

        if keyboard.vkCode == configured_key {
            if released {
                CONFIGURED_KEY_DOWN.store(false, Ordering::Release);
            } else if pressed {
                let initial_press = !CONFIGURED_KEY_DOWN.swap(true, Ordering::AcqRel);
                if initial_press
                    && matches_unmodified_press(configured_key, keyboard.vkCode, modifiers_down())
                {
                    send_event(PassthroughHotkeyEvent::Toggle);
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
        hotkey: PassthroughHotkey,
    ) -> Result<(Self, Receiver<PassthroughHotkeyEvent>), String> {
        let instance_id = HOOK_INSTANCE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = mpsc::channel();
        CONFIGURED_KEY.store(virtual_key(hotkey), Ordering::Release);
        CONFIGURED_KEY_DOWN.store(false, Ordering::Release);
        {
            let state = HOOK_STATE.get_or_init(|| Mutex::new(HookState::default()));
            let mut state = match state.lock() {
                Ok(state) => state,
                Err(poisoned) => poisoned.into_inner(),
            };
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
}

impl Drop for PassthroughHotkeyHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.thread.take() {
            let _ = worker.join();
        }
        CONFIGURED_KEY_DOWN.store(false, Ordering::Release);
        clear_sender(self.instance_id);
    }
}

fn clear_sender(instance_id: u64) {
    let Some(state) = HOOK_STATE.get() else {
        return;
    };
    let mut state = match state.lock() {
        Ok(state) => state,
        Err(poisoned) => poisoned.into_inner(),
    };
    if state.instance_id == instance_id {
        state.sender = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passthrough_config_maps_to_the_original_windows_keys() {
        assert_eq!(virtual_key(PassthroughHotkey::Home), VK_HOME as u32);
        assert_eq!(virtual_key(PassthroughHotkey::Insert), VK_INSERT as u32);
        assert_eq!(virtual_key(PassthroughHotkey::F8), VK_F8 as u32);
        assert_eq!(virtual_key(PassthroughHotkey::F9), VK_F9 as u32);
    }

    #[test]
    fn passthrough_toggle_requires_the_configured_unmodified_key() {
        assert!(matches_unmodified_press(
            VK_HOME as u32,
            VK_HOME as u32,
            false
        ));
        assert!(!matches_unmodified_press(
            VK_HOME as u32,
            VK_INSERT as u32,
            false
        ));
        assert!(!matches_unmodified_press(
            VK_HOME as u32,
            VK_HOME as u32,
            true
        ));
    }
}
