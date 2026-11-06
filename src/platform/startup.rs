//! The Start with Windows Run key (SPEC §11).
//!
//! The per-user Run value `HKCU\Software\Microsoft\Windows\CurrentVersion\Run\breakpoint` is the
//! only truth: Start with Windows is on exactly while it exists, and `config.json` never stores
//! it. Its data is the exe's path in quotes.

use std::os::windows::ffi::OsStrExt;

use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_SZ, RRF_RT_ANY, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
};
use windows::core::{PCWSTR, Result, w};

const RUN: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Run");
const VALUE: PCWSTR = w!("breakpoint");

/// Whether Start with Windows is on: the Run value exists, whatever it holds.
pub fn is_on() -> bool {
    // SAFETY: both names are static wide strings; no data is read back.
    let found =
        unsafe { RegGetValueW(HKEY_CURRENT_USER, RUN, VALUE, RRF_RT_ANY, None, None, None) };
    found == ERROR_SUCCESS
}

/// Writes the Run value with this exe's current path, or deletes it.
pub fn set(on: bool) -> Result<()> {
    if !on {
        // SAFETY: both names are static wide strings.
        return match unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN, VALUE) } {
            ERROR_SUCCESS | ERROR_FILE_NOT_FOUND => Ok(()),
            e => e.ok(),
        };
    }
    let exe = std::env::current_exe()?;
    let data: Vec<u16> = std::iter::once(u16::from(b'"'))
        .chain(exe.as_os_str().encode_wide())
        .chain([u16::from(b'"'), 0])
        .collect();
    // SAFETY: `data` is a NUL-terminated wide string of exactly the byte length passed.
    unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            RUN,
            VALUE,
            REG_SZ.0,
            Some(data.as_ptr().cast()),
            (data.len() * size_of::<u16>()) as u32,
        )
    }
    .ok()
}

/// On every start: if the Run value exists, rewrites it with the current exe path, so moving
/// the exe does not break autostart. Best effort.
pub fn refresh() {
    if is_on() {
        let _ = set(true);
    }
}
