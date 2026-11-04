//! GetLastInputInfo, vetoes, and session and power notifications.
//!
//! Events arrive at [`Listener`], a hidden top-level window rather than a message-only one:
//! message-only windows get no broadcasts, and the suspend and resume `WM_POWERBROADCAST`s are
//! broadcasts (presence findings, "What this design will get wrong", item 13). The window
//! records each event with the moment it arrived and nudges the app, which drains them with
//! [`take_signals`] on its next poll, so an event that lands while the app is busy waits instead
//! of being lost.

use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::time::{Duration, Instant};

use breakpoint::core::presence::{self, Signal};
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSessionManager as MediaManager,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as PlaybackStatus,
};
use windows::Win32::Foundation::{HANDLE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Power::{
    CallNtPowerInformation, ES_DISPLAY_REQUIRED, EXECUTION_STATE, HPOWERNOTIFY,
    POWERBROADCAST_SETTING, PowerUserPresent, RegisterPowerSettingNotification,
    SystemExecutionState, UnregisterPowerSettingNotification,
};
use windows::Win32::System::RemoteDesktop::{
    NOTIFY_FOR_THIS_SESSION, WTSRegisterSessionNotification, WTSUnRegisterSessionNotification,
};
use windows::Win32::System::SystemInformation::GetTickCount;
use windows::Win32::System::SystemServices::{
    GUID_SESSION_DISPLAY_STATUS, GUID_SESSION_USER_PRESENCE, PowerMonitorOff, PowerMonitorOn,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
use windows::Win32::UI::Shell::{
    QUNS_BUSY, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN, SHQueryUserNotificationState,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DEVICE_NOTIFY_WINDOW_HANDLE, DefWindowProcW, DestroyWindow, KillTimer,
    PBT_APMRESUMEAUTOMATIC, PBT_APMRESUMESUSPEND, PBT_APMSUSPEND, PBT_POWERSETTINGCHANGE,
    RegisterClassW, SetTimer, WM_POWERBROADCAST, WM_TIMER, WM_WTSSESSION_CHANGE, WNDCLASSW,
    WS_EX_TOOLWINDOW, WS_POPUP, WTS_CONSOLE_CONNECT, WTS_CONSOLE_DISCONNECT, WTS_REMOTE_CONNECT,
    WTS_REMOTE_DISCONNECT, WTS_SESSION_LOCK, WTS_SESSION_UNLOCK,
};
use windows::core::{Result, w};

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

/// The vetoes of SPEC §4, asked by the core only once input has stopped long enough.
#[derive(Default)]
pub struct Vetoes {
    /// The media-controls manager, kept after the first request succeeds.
    media: Option<MediaManager>,
}

impl presence::Vetoes for Vetoes {
    fn shell_busy(&mut self) -> bool {
        // SAFETY: no arguments.
        let state = unsafe { SHQueryUserNotificationState() };
        matches!(
            state,
            Ok(QUNS_BUSY | QUNS_RUNNING_D3D_FULL_SCREEN | QUNS_PRESENTATION_MODE)
        )
    }

    fn display_required(&mut self) -> bool {
        let mut state = EXECUTION_STATE(0);
        // SAFETY: the output buffer is one EXECUTION_STATE (a ULONG), as SystemExecutionState
        // fills, and its size is passed with it.
        let status = unsafe {
            CallNtPowerInformation(
                SystemExecutionState,
                None,
                0,
                Some(&mut state as *mut EXECUTION_STATE as *mut c_void),
                size_of::<EXECUTION_STATE>() as u32,
            )
        };
        status.is_ok() && state.contains(ES_DISPLAY_REQUIRED)
    }

    fn media_playing(&mut self) -> Option<bool> {
        self.any_media_playing().ok()
    }
}

impl Vetoes {
    /// Whether any session in the Global System Media Transport Controls is playing.
    fn any_media_playing(&mut self) -> Result<bool> {
        let manager = match &self.media {
            Some(manager) => manager.clone(),
            None => self
                .media
                .insert(MediaManager::RequestAsync()?.join()?)
                .clone(),
        };
        for session in manager.GetSessions()? {
            if session.GetPlaybackInfo()?.PlaybackStatus()? == PlaybackStatus::Playing {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

const RETRY_TIMER: usize = 1;
/// How often and how many times to retry the session registration: early in logon it fails
/// with `RPC_S_INVALID_BINDING` until Remote Desktop Services is up. Five minutes in all.
const RETRY_EVERY_MS: u32 = 5_000;
const RETRY_TIMES: u32 = 60;

thread_local! {
    static SIGNALS: RefCell<Vec<(Instant, Signal)>> = const { RefCell::new(Vec::new()) };
    static ON_SIGNAL: Cell<Option<fn()>> = const { Cell::new(None) };
    static RETRIES: Cell<u32> = const { Cell::new(0) };
}

/// The session and power events received since the last call, each with when it arrived.
pub fn take_signals() -> Vec<(Instant, Signal)> {
    SIGNALS.take()
}

/// The hidden window registered for session and power events. Lives on the UI thread.
pub struct Listener {
    hwnd: HWND,
    power: Vec<HPOWERNOTIFY>,
}

impl Listener {
    /// Starts listening; `on_signal` runs on the UI thread after each event is queued.
    pub fn new(on_signal: fn()) -> Result<Self> {
        ON_SIGNAL.set(Some(on_signal));
        // SAFETY: plain Win32 window creation and registration for the window just created.
        unsafe {
            let instance = GetModuleHandleW(None)?.into();
            let class = w!("breakpoint.presence");
            RegisterClassW(&WNDCLASSW {
                lpfnWndProc: Some(wndproc),
                hInstance: instance,
                lpszClassName: class,
                ..Default::default()
            });
            // Top-level so the power broadcasts reach it; never shown.
            let hwnd = CreateWindowExW(
                WS_EX_TOOLWINDOW,
                class,
                w!("breakpoint"),
                WS_POPUP,
                0,
                0,
                0,
                0,
                None,
                None,
                Some(instance),
                None,
            )?;
            if WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION).is_err() {
                SetTimer(Some(hwnd), RETRY_TIMER, RETRY_EVERY_MS, None);
            }
            // Without these breakpoint still works on input and the other events alone.
            let power = [GUID_SESSION_DISPLAY_STATUS, GUID_SESSION_USER_PRESENCE]
                .iter()
                .filter_map(|guid| {
                    RegisterPowerSettingNotification(
                        HANDLE(hwnd.0),
                        guid,
                        DEVICE_NOTIFY_WINDOW_HANDLE,
                    )
                    .ok()
                })
                .collect();
            Ok(Listener { hwnd, power })
        }
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        // SAFETY: the handles are the ones registered in `new` for this live window. The
        // session unregistration fails harmlessly if the registration never succeeded.
        unsafe {
            for &handle in &self.power {
                let _ = UnregisterPowerSettingNotification(handle);
            }
            let _ = WTSUnRegisterSessionNotification(self.hwnd);
            let _ = DestroyWindow(self.hwnd);
        }
        ON_SIGNAL.set(None);
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let signal = match msg {
        WM_WTSSESSION_CHANGE => session_signal(wparam.0 as u32),
        WM_POWERBROADCAST => {
            // SAFETY: for PBT_POWERSETTINGCHANGE, lParam points to a POWERBROADCAST_SETTING
            // that is valid for the duration of this message.
            let signal = unsafe { power_signal(wparam.0 as u32, lparam) };
            queue(signal);
            // "An application should return TRUE if it processes this message."
            return LRESULT(1);
        }
        WM_TIMER if wparam.0 == RETRY_TIMER => {
            retry_session_registration(hwnd);
            None
        }
        // SAFETY: forwarding the arguments this procedure was called with.
        _ => return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    };
    queue(signal);
    LRESULT(0)
}

fn queue(signal: Option<Signal>) {
    let Some(signal) = signal else { return };
    SIGNALS.with_borrow_mut(|signals| signals.push((Instant::now(), signal)));
    if let Some(on_signal) = ON_SIGNAL.get() {
        on_signal();
    }
}

fn session_signal(change: u32) -> Option<Signal> {
    match change {
        WTS_SESSION_LOCK => Some(Signal::Lock),
        WTS_SESSION_UNLOCK => Some(Signal::Unlock),
        WTS_CONSOLE_DISCONNECT | WTS_REMOTE_DISCONNECT => Some(Signal::Disconnect),
        WTS_CONSOLE_CONNECT | WTS_REMOTE_CONNECT => Some(Signal::Connect),
        _ => None,
    }
}

/// # Safety
///
/// For `PBT_POWERSETTINGCHANGE`, `lparam` must point to a valid `POWERBROADCAST_SETTING`.
unsafe fn power_signal(event: u32, lparam: LPARAM) -> Option<Signal> {
    match event {
        PBT_APMSUSPEND => Some(Signal::Suspend),
        PBT_APMRESUMESUSPEND => Some(Signal::Resume),
        PBT_APMRESUMEAUTOMATIC => Some(Signal::ResumeAutomatic),
        PBT_POWERSETTINGCHANGE => {
            // SAFETY: guaranteed by the caller.
            let setting = unsafe { &*(lparam.0 as *const POWERBROADCAST_SETTING) };
            if setting.DataLength < size_of::<u32>() as u32 {
                return None;
            }
            // SAFETY: Data holds DataLength bytes, at least four, not necessarily aligned.
            let value = unsafe { setting.Data.as_ptr().cast::<i32>().read_unaligned() };
            match setting.PowerSetting {
                GUID_SESSION_DISPLAY_STATUS if value == PowerMonitorOff.0 => {
                    Some(Signal::DisplayOff)
                }
                GUID_SESSION_DISPLAY_STATUS if value == PowerMonitorOn.0 => Some(Signal::DisplayOn),
                GUID_SESSION_USER_PRESENCE if value == PowerUserPresent.0 => {
                    Some(Signal::UserPresent)
                }
                // PowerUserInactive, and PowerUserNotPresent should it ever come.
                GUID_SESSION_USER_PRESENCE => Some(Signal::UserInactive),
                // Dimmed is neither: Windows dims before it turns the display off.
                _ => None,
            }
        }
        _ => None,
    }
}

fn retry_session_registration(hwnd: HWND) {
    let tries = RETRIES.get() + 1;
    RETRIES.set(tries);
    // SAFETY: `hwnd` is the listener's live window, which owns the timer.
    unsafe {
        if WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION).is_ok()
            || tries >= RETRY_TIMES
        {
            let _ = KillTimer(Some(hwnd), RETRY_TIMER);
        }
    }
}
