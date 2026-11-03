//! Local time and UTC offset.

use breakpoint::core::time::{Civil, LocalTime};
use windows::Win32::Foundation::SYSTEMTIME;
use windows::Win32::System::SystemInformation::GetSystemTime;
use windows::Win32::System::Time::SystemTimeToTzSpecificLocalTime;

/// Now, with the offset Windows' current time zone has at this moment (daylight saving
/// included). If the conversion fails the offset is zero.
pub fn now() -> LocalTime {
    // SAFETY: both SYSTEMTIMEs are valid for the call; `None` means the current time zone.
    let (utc, local) = unsafe {
        let utc = GetSystemTime();
        let mut local = utc;
        if SystemTimeToTzSpecificLocalTime(None, &utc, &mut local).is_err() {
            local = utc;
        }
        (utc, local)
    };
    LocalTime::from_utc_and_local(civil(utc), civil(local))
}

fn civil(t: SYSTEMTIME) -> Civil {
    Civil {
        year: t.wYear.into(),
        month: t.wMonth as u8,
        day: t.wDay as u8,
        hour: t.wHour as u8,
        minute: t.wMinute as u8,
        second: t.wSecond as u8,
    }
}
