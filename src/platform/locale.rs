//! Windows system-locale lookup, used to pick a sensible default UI language the
//! first time the app runs (before any `config.json` exists).

use windows_sys::Win32::Globalization::GetUserDefaultLocaleName;

/// Buffer size the Windows SDK documents for `GetUserDefaultLocaleName`
/// (`LOCALE_NAME_MAX_LENGTH`).
const LOCALE_NAME_MAX_LENGTH: usize = 85;

/// The signed-in user's Windows locale name (e.g. `"zh-CN"`, `"ja-JP"`, `"en-US"`), or
/// `None` if the API call fails.
pub fn system_locale_name() -> Option<String> {
    let mut buffer = [0u16; LOCALE_NAME_MAX_LENGTH];
    // SAFETY: `buffer` is sized to `LOCALE_NAME_MAX_LENGTH`, the exact bound the API
    // documents for this call, so the write can never overrun it.
    let length = unsafe { GetUserDefaultLocaleName(buffer.as_mut_ptr(), buffer.len() as i32) };
    // A length of 0 means the call failed; 1 is just the null terminator (empty name).
    if length <= 1 {
        return None;
    }
    Some(String::from_utf16_lossy(&buffer[..length as usize - 1]))
}
