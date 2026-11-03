//! GetLastInputInfo, vetoes, and session and power notifications.

use std::time::Duration;

use windows::Win32::System::SystemInformation::GetTickCount;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};

/// The time of the last keyboard or mouse input in this session, as an offset on the core's
/// timeline given that it reads `now` at this moment.
pub fn last_input(now: Duration) -> Duration {
    let mut info = LASTINPUTINFO {
        cbSize: size_of::<LASTINPUTINFO>() as u32,
        dwTime: 0,
    };
    // SAFETY: `info` is a valid LASTINPUTINFO with cbSize set, as the call requires.
    let idle_ms = unsafe {
        if !GetLastInputInfo(&mut info).as_bool() {
            return now;
        }
        // Both are 32-bit tick counts, so the difference survives the 49-day wrap.
        GetTickCount().wrapping_sub(info.dwTime)
    };
    now.saturating_sub(Duration::from_millis(idle_ms.into()))
}
