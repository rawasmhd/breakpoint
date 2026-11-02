# Activity and presence detection on Windows

Resolves [#25](https://github.com/rawasmhd/breakpoint/issues/25). Feeds **How a nudge is satisfied**.

Question: how reliably can breakpoint tell that the user has *actually stepped away* from the
computer, versus being still at the keyboard, so a Stand nudge can be satisfied by observed
behaviour (no input for 60 to 120 s) rather than a button press?

Sources are primary only: Microsoft Learn Win32 / WinRT references, Microsoft support and
hardware-design pages, and the `windows` crate documentation (0.62.2, the newest published
version on docs.rs at time of writing, released 2025-10-06:
https://docs.rs/crate/windows/latest). Every claim carries its URL.

---

## TL;DR

- **Windows gives no "user is physically present" signal on ordinary hardware.** Everything
  is a proxy for *input*. The one true presence sensor (`HumanPresenceSensor`) is optional OEM
  hardware on a minority of Windows 11 machines and is gated by a privacy toggle.
- **`GetLastInputInfo` is the workhorse**: cheap, per-session, 10–16 ms resolution, fed by
  keyboard, mouse, touch and pen. It cannot distinguish a hardware mouse jiggler from a
  human; it *does* count `SendInput` (software jigglers); it does *not* count media playback.
- **Hard "away" evidence exists and is free**: session lock (`WM_WTSSESSION_CHANGE`), display
  turned off by the idle timer (`GUID_SESSION_DISPLAY_STATUS`), Windows' own "user inactive"
  verdict (`GUID_SESSION_USER_PRESENCE`), suspend (`PBT_APMSUSPEND`), RDP/console disconnect.
- **"Present but not typing" is partly detectable**: full-screen apps and presentation mode
  via `SHQueryUserNotificationState`; active media via the Global System Media Transport
  Controls (GSMTC) session manager; any app holding the display awake via
  `CallNtPowerInformation(SystemExecutionState)`. Windowed video in a browser that does not
  integrate with SMTC is the gap.
- **Recommended design**: idle timer from `GetLastInputInfo` polled every few seconds, with
  *vetoes* (media playing, display-required, full-screen) and *accelerators* (lock, display
  off, suspend, disconnect) delivered to a hidden message-only window. Details in
  [Recommended design](#recommended-design-for-breakpoint).

---

## 1. `GetLastInputInfo`

Reference: https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getlastinputinfo
and https://learn.microsoft.com/en-us/windows/win32/api/winuser/ns-winuser-lastinputinfo

### What it reports

`LASTINPUTINFO.dwTime` is "The tick count when the last input event was received"
(https://learn.microsoft.com/en-us/windows/win32/api/winuser/ns-winuser-lastinputinfo).
Idle time is therefore `GetTickCount() - dwTime`, computed with wrapping arithmetic.

### Resolution and wrap-around

Both values are system tick counts. "The resolution of the GetTickCount function is limited
to the resolution of the system timer, which is typically in the range of 10 milliseconds to
16 milliseconds" and "the time will wrap around to zero if the system is run continuously for
49.7 days"
(https://learn.microsoft.com/en-us/windows/win32/api/sysinfoapi/nf-sysinfoapi-gettickcount).
For a 60–120 s threshold the resolution is irrelevant; the wrap is handled by `u32::wrapping_sub`.

The value is also **not guaranteed monotonic**: "The tick count when the last input event was
received ... is not guaranteed to be incremental. In some cases, the value might be less than
the tick count of a prior event. For example, this can be caused by a timing gap between the
raw input thread and the desktop thread or an event raised by SendInput, which supplies its own
tick count" (https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getlastinputinfo).
Treat each poll independently; never assume the value only moves forward.

### Multiple sessions and Remote Desktop

"GetLastInputInfo does not provide system-wide user input information across all running
sessions. Rather, GetLastInputInfo provides session-specific user input information for only
the session that invoked the function"
(https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getlastinputinfo).

Consequences:

- **Fast User Switching**: when another user's session is active, breakpoint's session
  receives no input and looks idle. That is *correct* for a stand nudge (our user is not at
  this keyboard), but to avoid firing nudges into a background session, watch for
  `WTS_CONSOLE_DISCONNECT` / `WTS_CONSOLE_CONNECT` (section 2).
- **Remote Desktop**: an RDP connection reconnects the *same* session to a remote terminal
  (`WTS_REMOTE_CONNECT`), so input arriving over RDP is input to our session and counts.
  `GetSystemMetrics(SM_REMOTESESSION)` is nonzero "If the calling process is associated with a
  Terminal Services client session"
  (https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getsystemmetrics),
  so breakpoint can tell it is being driven remotely. When the RDP client disconnects
  (`WTS_REMOTE_DISCONNECT`) the session keeps running with no input, so the idle counter climbs
  even though the user may be busy at the *client* machine. The user is not at *this* desk,
  so treat disconnect as "away", not as "standing".
- **Cross-session alternative**: `WTSQuerySessionInformation(WTSSessionInfoEx)` returns a
  `WTSINFOEX_LEVEL1` whose `LastInputTime` is "The time of the last user input in the session"
  as a FILETIME-style value, plus `SessionFlags` (`WTS_SESSIONSTATE_LOCK` / `UNLOCK`)
  (https://learn.microsoft.com/en-us/windows/win32/api/wtsapi32/ns-wtsapi32-wtsinfoex_level1_w).
  Note the documented defect: on Windows 7 / Server 2008 R2 the lock and unlock flag values are
  reversed (same page). Useful as a polled fallback for lock state; not needed for idle time
  in our own session.

### Does media playback count?

No. The power manager's own description of what is considered activity: "The system
automatically detects activities such as local keyboard or mouse input, server activity, and
changing window focus. Activities that are not automatically detected include disk or CPU
activity and video display." Media players are instead told to call
`SetThreadExecutionState(ES_DISPLAY_REQUIRED)`: "Multimedia applications, such as video players
and presentation applications, must use ES_DISPLAY_REQUIRED when they display video for long
periods of time without user input"
(https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-setthreadexecutionstate).
So a user watching a video shows up as idle to `GetLastInputInfo`; see section 6 for how to
detect the playback instead.

### Does a mouse jiggler count?

- **Hardware jigglers** (USB HID devices that emit real mouse reports) are indistinguishable
  from a human mouse. Nothing in the input stack flags them. Not detectable.
- **Software jigglers** typically use `SendInput`, which "inserts the events in the INPUT
  structures serially into the keyboard or mouse input stream"
  (https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-sendinput). The
  `GetLastInputInfo` page explicitly names `SendInput` as a source of last-input tick counts
  (https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getlastinputinfo),
  so synthesised input resets the idle timer. The only documented limit on `SendInput` is
  UIPI: "Applications are permitted to inject input only into applications that are at an
  equal or lesser integrity level" (SendInput page above), which does not help us.

Conclusion: a user who wants to defeat the Stand nudge can, and breakpoint should not try to
stop them. It is a nudge, not a time clock.

---

## 2. Session lock / unlock: `WTSRegisterSessionNotification`

Reference: https://learn.microsoft.com/en-us/windows/win32/api/wtsapi32/nf-wtsapi32-wtsregistersessionnotification
and https://learn.microsoft.com/en-us/windows/win32/termserv/wm-wtssession-change

- Register a window with `WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION)`.
  "Session change notifications are sent in the form of a WM_WTSSESSION_CHANGE message. These
  notifications are sent only to the windows that have registered for them using this
  function." Because they are *sent to the registered window*, not broadcast, a message-only
  window works (see section 3 on broadcast vs. direct delivery).
- "For every call to this function, there must be a corresponding call to
  WTSUnRegisterSessionNotification" before the window is destroyed.
- Startup race: "If this function is called before the dependent services of Remote Desktop
  Services have started, an RPC_S_INVALID_BINDING error code may be returned." breakpoint
  starts at logon, so retry with backoff if registration fails.
- `wParam` values that matter (all from the WM_WTSSESSION_CHANGE page):
  `WTS_SESSION_LOCK` (0x7), `WTS_SESSION_UNLOCK` (0x8), `WTS_CONSOLE_CONNECT` (0x1),
  `WTS_CONSOLE_DISCONNECT` (0x2), `WTS_REMOTE_CONNECT` (0x3), `WTS_REMOTE_DISCONNECT` (0x4),
  `WTS_SESSION_LOGOFF` (0x6). `lParam` is the session id.
- Minimum OS: Windows Vista.

Lock is the strongest cheap "away" signal: the user pressed Win+L or Windows locked after the
display timeout. Treat `WTS_SESSION_LOCK` as instant satisfaction of a pending Stand nudge and
`WTS_SESSION_UNLOCK` as the start of a new sitting block.

---

## 3. Sleep / resume and display state: `WM_POWERBROADCAST`

Reference: https://learn.microsoft.com/en-us/windows/win32/power/wm-powerbroadcast

### Events

| `wParam` | Meaning (verbatim from the page) |
|---|---|
| `PBT_APMSUSPEND` (4) | "System is suspending operation." |
| `PBT_APMRESUMEAUTOMATIC` (18) | "Operation is resuming automatically from a low-power state. This message is sent every time the system resumes." |
| `PBT_APMRESUMESUSPEND` (7) | "Operation is resuming from a low-power state. This message is sent after PBT_APMRESUMEAUTOMATIC if the resume is triggered by user input, such as pressing a key." |
| `PBT_POWERSETTINGCHANGE` (32787) | "A power setting change event has been received." `lParam` points to a `POWERBROADCAST_SETTING`. |

"WM_POWERBROADCAST messages do not distinguish between different low-power states" (same page).
`PBT_APMRESUMEAUTOMATIC` "does not indicate whether a user is present"; the later
`PBT_APMRESUMESUSPEND` is the one that means a human woke the machine
(https://learn.microsoft.com/en-us/windows/win32/power/pbt-apmresumeautomatic). Edge case
from the same page: "In Windows 10, version 1507 systems and later, if the system is resuming
from sleep only to immediately enter hibernation, then this event isn't delivered."

### What a hidden message-only window needs

A message-only window "is not visible, has no z-order, cannot be enumerated, and does not
receive broadcast messages. The window simply dispatches messages"
(https://learn.microsoft.com/en-us/windows/win32/winmsg/window-features). It is created by
passing `HWND_MESSAGE` as `hWndParent` to `CreateWindowEx`
(https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-createwindowexw).

That matters because the plain suspend/resume `WM_POWERBROADCAST` is a broadcast to top-level
windows. Two ways to receive power events on a message-only window, both of which target a
specific `HWND`:

1. **`RegisterSuspendResumeNotification(hwnd, DEVICE_NOTIFY_WINDOW_HANDLE)`** (Windows 8+):
   "Registers to receive notification when the system is suspended or resumed ... operates in
   user mode and can take a window handle." Events delivered are `PBT_APMSUSPEND`,
   `PBT_APMRESUMESUSPEND`, `PBT_APMRESUMEAUTOMATIC`
   (https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-registersuspendresumenotification).
2. **`RegisterPowerSettingNotification(hwnd, &GUID, DEVICE_NOTIFY_WINDOW_HANDLE)`**
   (Vista+): "Notifications are sent using WM_POWERBROADCAST messages with a wParam parameter
   of PBT_POWERSETTINGCHANGE"
   (https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-registerpowersettingnotification).
   Unregister with `UnregisterPowerSettingNotification`
   (https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-unregisterpowersettingnotification).

If breakpoint's tray icon already requires an ordinary (invisible, non-message-only) top-level
window, the broadcasts arrive anyway; but registering explicitly is cheap and makes the design
independent of that detail. **Verify at runtime** that a message-only window registered via
(1) actually receives `PBT_APMSUSPEND`; Microsoft documents the delivery target but not this
specific combination.

### Power-setting GUIDs worth registering

From https://learn.microsoft.com/en-us/windows/win32/power/power-setting-guids (all Windows 8+
unless noted):

- **`GUID_SESSION_DISPLAY_STATUS`** — "The display associated with the application's session
  has been powered on or off ... sent only to user-mode applications." Data is
  `MONITOR_DISPLAY_STATE`: `PowerMonitorOff` (0), `PowerMonitorOn` (1), `PowerMonitorDim` (2).
  "All applications that run in an interactive user-mode session should use this setting"
  rather than `GUID_CONSOLE_DISPLAY_STATE`. Display-off after the idle timeout is a strong
  away signal; `Dim` is Windows saying "nobody has touched this for a while".
- **`GUID_SESSION_USER_PRESENCE`** — "The user status associated with the application's
  session has changed ... sent only to user-mode applications running in an interactive
  session." Data is `USER_ACTIVITY_PRESENCE`: `PowerUserPresent` (0) "The user is providing
  input to the session", `PowerUserInactive` (2) "The user activity timeout has elapsed with
  no interaction from the user". (Enum reference:
  https://learn.microsoft.com/en-us/windows/win32/api/winnt/ne-winnt-user_activity_presence.)
  This is Windows' own idle verdict, driven by the same input the power manager watches, and
  it is pushed rather than polled. Its timeout is the user's power-plan setting, not ours, so
  it complements rather than replaces our own 60–120 s timer.
- `GUID_MONITOR_POWER_ON` is the Vista-era equivalent; "New applications should use
  GUID_CONSOLE_DISPLAY_STATE instead" (same page).
- `GUID_LIDSWITCH_STATE_CHANGE` — lid closed (0) / opened (1). On a laptop, lid closed is
  "away" by definition.

The payload is a `POWERBROADCAST_SETTING { GUID PowerSetting; DWORD DataLength; UCHAR Data[1]; }`
(https://learn.microsoft.com/en-us/windows/win32/api/winuser/ns-winuser-powerbroadcast_setting).

---

## 4. Screen-saver state

`SystemParametersInfo(SPI_GETSCREENSAVERRUNNING)` "Determines whether a screen saver is
currently running on the window station of the calling process ... Note that only the
interactive window station, WinSta0, can have a screen saver running"
(https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-systemparametersinfow).
There is no push notification; it must be polled. `SPI_GETSCREENSAVEACTIVE` (is a screen saver
*configured*) is documented as returning TRUE on Windows 7 "even when screen saving is not
enabled" (same page), so do not rely on it.

Simpler: `SHQueryUserNotificationState` returns `QUNS_NOT_PRESENT` when "A screen saver is
displayed, the machine is locked, or a nonactive Fast User Switching session is in progress"
(https://learn.microsoft.com/en-us/windows/win32/api/shellapi/ne-shellapi-query_user_notification_state),
which folds the screen-saver case into one call we are already making (section 6). Screen
savers are a weak signal in practice because they are an opt-in feature; treat as a bonus,
not a dependency.

---

## 5. Presence sensors and webcams

### `Windows.Devices.Sensors.HumanPresenceSensor`

Reference: https://learn.microsoft.com/en-us/uwp/api/windows.devices.sensors.humanpresencesensor

- "Represents a sensor that detects whether a user is present, absent, or not interacting
  with their device." Requirements table: "Windows 11 Insider Preview (introduced in
  10.0.23504.0)", `UniversalApiContract` v15; the page's version selector lists it from the
  22621 (22H2) SDK onward.
- API surface: `GetDefault()` / `GetDefaultAsync()` return "A reference to a human presence
  sensor" (https://learn.microsoft.com/en-us/uwp/api/windows.devices.sensors.humanpresencesensor.getdefault);
  when the machine has no sensor this is null, which `windows-rs` surfaces as an `Err`.
  `GetCurrentReading()` returns a `HumanPresenceSensorReading` with `Presence`
  (`Unknown` 0 / `Present` 1 / `NotPresent` 2 —
  https://learn.microsoft.com/en-us/uwp/api/windows.devices.sensors.humanpresence),
  `Engagement`, `DistanceInMillimeters`
  (https://learn.microsoft.com/en-us/uwp/api/windows.devices.sensors.humanpresencesensorreading),
  and a `ReadingChanged` event "whenever there is a significant change".
- **Hardware availability**: these are OEM-optional components. Microsoft's own support page
  says only "Some Windows 11 PCs have a presence sensor built in"
  (https://support.microsoft.com/en-US/Windows/privacy/managing-presence-sensing-settings-in-windows-11).
  The OEM guideline describes them as "proximity sensors of type 'Human'", a "minimum viable
  sensor setup" with a 0.2–1.2 m range, and explicitly requires that "Human presence sensors
  must be backed by a physical device"
  (https://learn.microsoft.com/en-us/windows-hardware/design/component-guidelines/sensors-presence-sensors).
- **Privacy gating**: the `humanPresence` device capability "is required to use some APIs in
  the Windows.Devices.Sensors namespace" for packaged apps
  (https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/app-capability-declarations),
  and for unpackaged apps like breakpoint the user controls access through Settings > Privacy
  & security > Presence Sensing: "Desktop apps cannot be individually toggled, but access for
  those apps can be controlled as a group" via "Let desktop apps access Presence Sensing"
  (support page above).

Verdict: **not a dead end, but not a foundation**. On a machine that has one and where the
toggle is on, `Presence == NotPresent` is the best away signal available and should be used
opportunistically. breakpoint must work identically when `GetDefault` yields nothing, which
will be the common case on desktops and most laptops.

### Webcam-based presence

Dead end for breakpoint. Nothing in the OS offers "is a face in frame" to desktop apps; it
would mean capturing frames, running a detector, keeping the camera LED lit, and spending CPU
continuously, all for a tray app whose brief is to be tiny. Microsoft's hardware guidance
points the same way: it requires presence sensing to run where "image or audio metadata is not
accessible to applications on the OS" and states that OEMs who process image metadata on the
OS "take on all legal liability for the privacy of such implementation"
(https://learn.microsoft.com/en-us/windows-hardware/design/component-guidelines/sensors-presence-sensors).

---

## 6. "Present but not typing" (watching a video)

Three independent signals, each with a gap.

### `SHQueryUserNotificationState`

Reference: https://learn.microsoft.com/en-us/windows/win32/api/shellapi/nf-shellapi-shqueryusernotificationstate
and https://learn.microsoft.com/en-us/windows/win32/api/shellapi/ne-shellapi-query_user_notification_state

Returns one `QUERY_USER_NOTIFICATION_STATE`:

| Value | Meaning (verbatim) | Use for breakpoint |
|---|---|---|
| `QUNS_NOT_PRESENT` (1) | "A screen saver is displayed, the machine is locked, or a nonactive Fast User Switching session is in progress." | away |
| `QUNS_BUSY` (2) | "A full-screen application is running or Presentation Settings are applied." | present, do not count idle |
| `QUNS_RUNNING_D3D_FULL_SCREEN` (3) | "A full-screen (exclusive mode) Direct3D application is running." | present (gaming), suppress nudge |
| `QUNS_PRESENTATION_MODE` (4) | "The user has activated Windows presentation settings to block notifications and pop-up messages." | present, suppress nudge |
| `QUNS_ACCEPTS_NOTIFICATIONS` (5) | "None of the other states are found, notifications can be freely sent." | normal |
| `QUNS_QUIET_TIME` (6) | first hour after a new account / OS upgrade | normal |
| `QUNS_APP` (7) | "A Windows Store app is running." (Windows 8 full-screen app) | present |

Limits: "there are no notifications sent when the user starts or stops a full-screen
application" (function page), so poll it alongside `GetLastInputInfo`. It catches full-screen
video and presentations only; a YouTube tab in a normal browser window is invisible to it.
Vista+, Shell32.

### Media session APIs (GSMTC)

Reference: https://learn.microsoft.com/en-us/uwp/api/windows.media.control.globalsystemmediatransportcontrolssessionmanager

`GlobalSystemMediaTransportControlsSessionManager` "Provides access to playback sessions
throughout the system that have integrated with SystemMediaTransportControls". Windows 10
1809+. `RequestAsync()` gives the manager; `GetSessions()` all sessions; each session's
`GetPlaybackInfo().PlaybackStatus` is `Closed` / `Opened` / `Changing` / `Stopped` /
`Playing` / `Paused`
(https://learn.microsoft.com/en-us/uwp/api/windows.media.control.globalsystemmediatransportcontrolssessionplaybackstatus).

Limits:

- Only apps that integrate with SMTC appear. Modern browsers and media players mostly do (it
  is what drives the Windows volume-flyout media card), but nothing guarantees it.
- `Playing` cannot tell video from audio. Music while working is normal; it must not veto
  idle detection on its own, only in combination with no input *and* display-required.
- The `globalMediaControl` capability "is required to use some APIs in the
  Windows.Media.Control namespace" for packaged apps
  (https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/app-capability-declarations).
  The same page says "Most scenarios for app capabilities are relevant only to apps that have
  package identity", which breakpoint does not have. **Verify at runtime** that `RequestAsync`
  succeeds from an unpackaged process; if it fails, drop the signal, do not add packaging.

### Display-required requests: `CallNtPowerInformation(SystemExecutionState)`

Reference: https://learn.microsoft.com/en-us/windows/win32/api/powerbase/nf-powerbase-callntpowerinformation

`SystemExecutionState` "receives a ULONG value containing the system execution state buffer.
This value may contain any combination of the following values: ES_SYSTEM_REQUIRED,
ES_DISPLAY_REQUIRED, or ES_USER_PRESENT." Because video players and presentation apps are
documented to set `ES_DISPLAY_REQUIRED` (section 1), a set bit means *something on this machine
is deliberately keeping the screen on*. It is the same information `powercfg /requests` shows.
XP+, PowrProf.dll.

Limits: it is system-wide, so another user's session or a background process can hold it; it
does not say *which* app; and it is a request, not proof of viewing.

### Windows' own verdict: `GUID_SESSION_USER_PRESENCE`

Already covered in section 3. `PowerUserInactive` fires when the power manager's activity
timeout has elapsed, which already accounts for display-required requests. If breakpoint sees
`PowerUserInactive` it can trust that Windows itself considers the user gone.

---

## 7. How the `windows` crate exposes each API

Crate: `windows` 0.62.2 (https://docs.rs/crate/windows/latest). Features follow the module
path with `_` separators; the docs.rs feature list for 0.62.2 confirms
`Win32_UI_Input_KeyboardAndMouse`, `Win32_System_RemoteDesktop`,
`Win32_System_SystemInformation`, `Devices_Sensors`, `Media_Control`, `Foundation`
(https://docs.rs/crate/windows/latest/features); the remaining names below follow the same
rule. Module paths and signatures below were read from the 0.62.2 API docs at
https://microsoft.github.io/windows-docs-rs/doc/windows/.

Heads-up: the `windows-rs` main branch readme already shows a future `windows = "0.100"` whose
Win32 features "follow header groups, such as `handleapi` and `synchapi`" instead of
namespaces (https://github.com/microsoft/windows-rs/blob/master/crates/libs/windows/readme.md).
Pin to 0.62 and expect to rename features once 0.100 ships.

```toml
[dependencies.windows]
version = "0.62"
features = [
  "Win32_Foundation",
  "Win32_UI_Input_KeyboardAndMouse",   # GetLastInputInfo, LASTINPUTINFO
  "Win32_System_SystemInformation",    # GetTickCount
  "Win32_System_RemoteDesktop",        # WTSRegisterSessionNotification, WTSQuerySessionInformationW
  "Win32_UI_WindowsAndMessaging",      # CreateWindowExW, HWND_MESSAGE, WM_*, PBT_*, SPI_*, SM_*
  "Win32_System_Power",                # RegisterPowerSettingNotification, CallNtPowerInformation, ES_*
  "Win32_System_SystemServices",       # GUID_SESSION_DISPLAY_STATUS, GUID_SESSION_USER_PRESENCE
  "Win32_UI_Shell",                    # SHQueryUserNotificationState
  "Devices_Sensors",                   # HumanPresenceSensor (WinRT)
  "Media_Control",                     # GlobalSystemMediaTransportControlsSessionManager (WinRT)
  "Foundation", "Foundation_Collections",
]
```

Signatures (0.62.2):

| API | Rust path and signature |
|---|---|
| `GetLastInputInfo` | `Win32::UI::Input::KeyboardAndMouse::GetLastInputInfo(plii: *mut LASTINPUTINFO) -> BOOL`; `LASTINPUTINFO { cbSize: u32, dwTime: u32 }` |
| `GetTickCount` | `Win32::System::SystemInformation::GetTickCount() -> u32` |
| `WTSRegisterSessionNotification` | `Win32::System::RemoteDesktop::WTSRegisterSessionNotification(hwnd: HWND, dwflags: u32) -> Result<()>`; `NOTIFY_FOR_THIS_SESSION: u32 = 0` |
| `WM_WTSSESSION_CHANGE` | `Win32::UI::WindowsAndMessaging::WM_WTSSESSION_CHANGE: u32 = 689` |
| `WTSQuerySessionInformationW` | `Win32::System::RemoteDesktop::WTSQuerySessionInformationW(hserver: Option<HANDLE>, sessionid: u32, wtsinfoclass: WTS_INFO_CLASS, ppbuffer: *mut PWSTR, pbytesreturned: *mut u32) -> Result<()>`; `WTSSessionInfoEx`, `WTS_CURRENT_SESSION: u32 = 0xFFFFFFFF`, `WTSINFOEXW { Level: u32, Data: WTSINFOEX_LEVEL_W }` |
| `RegisterSuspendResumeNotification` | `Win32::System::Power::RegisterSuspendResumeNotification(hrecipient: HANDLE, flags: REGISTER_NOTIFICATION_FLAGS) -> Result<HPOWERNOTIFY>` |
| `RegisterPowerSettingNotification` | `Win32::System::Power::RegisterPowerSettingNotification(hrecipient: HANDLE, powersettingguid: *const GUID, flags: REGISTER_NOTIFICATION_FLAGS) -> Result<HPOWERNOTIFY>`; `DEVICE_NOTIFY_WINDOW_HANDLE` and `REGISTER_NOTIFICATION_FLAGS(pub u32)` live in `Win32::UI::WindowsAndMessaging` |
| `PBT_*`, `POWERBROADCAST_SETTING`, `HWND_MESSAGE` | all in `Win32::UI::WindowsAndMessaging`; `PBT_APMSUSPEND: u32 = 4` |
| `GUID_CONSOLE_DISPLAY_STATE` etc. | `Win32::System::SystemServices::GUID_CONSOLE_DISPLAY_STATE: GUID` (siblings `GUID_SESSION_DISPLAY_STATUS`, `GUID_SESSION_USER_PRESENCE` follow the same path) |
| `SystemParametersInfoW` | `Win32::UI::WindowsAndMessaging::SystemParametersInfoW(uiaction: SYSTEM_PARAMETERS_INFO_ACTION, uiparam: u32, pvparam: Option<*mut c_void>, fwinini: SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS) -> Result<()>`; `SPI_GETSCREENSAVERRUNNING` |
| `SHQueryUserNotificationState` | `Win32::UI::Shell::SHQueryUserNotificationState() -> Result<QUERY_USER_NOTIFICATION_STATE>` |
| `CallNtPowerInformation` | `Win32::System::Power::CallNtPowerInformation(informationlevel: POWER_INFORMATION_LEVEL, inputbuffer: Option<*const c_void>, inputbufferlength: u32, outputbuffer: Option<*mut c_void>, outputbufferlength: u32) -> NTSTATUS`; `SystemExecutionState`, `ES_DISPLAY_REQUIRED: EXECUTION_STATE` |
| `GetSystemMetrics` | `Win32::UI::WindowsAndMessaging::GetSystemMetrics(nindex: SYSTEM_METRICS_INDEX) -> i32`; `SM_REMOTESESSION` |
| `HumanPresenceSensor` | `Devices::Sensors::HumanPresenceSensor::GetDefaultAsync() -> Result<IAsyncOperation<HumanPresenceSensor>>`, `GetCurrentReading(&self) -> Result<HumanPresenceSensorReading>`, `IsPresenceSupported(&self) -> Result<bool>`, `ReadingChanged(&self, handler) -> Result<i64>`; reading `.Presence() -> Result<HumanPresence>` |
| GSMTC | `Media::Control::GlobalSystemMediaTransportControlsSessionManager::RequestAsync() -> Result<IAsyncOperation<..Manager>>`, `GetSessions(&self) -> Result<IVectorView<..Session>>`, session `.GetPlaybackInfo() -> Result<..PlaybackInfo>`, `.SourceAppUserModelId() -> Result<HSTRING>` |
| `IAsyncOperation<T>::get` | `pub fn get(&self) -> Result<T>` — "Waits for the IAsyncOperation<T> to finish" (`windows-future`, https://docs.rs/windows-future/0.2.1/windows_future/struct.IAsyncOperation.html; 0.62.2 depends on `windows-future ^0.3.2`) |

### Minimal snippets

All `unsafe` Win32 calls are shown without error-handling sugar beyond `?`. WinRT calls need the
thread's apartment initialised first (the crate's own sample calls an MTA initialiser before
touching WinRT: https://github.com/microsoft/windows-rs/blob/master/crates/libs/windows/readme.md).

**Idle time (poll every few seconds)**

```rust
use windows::Win32::System::SystemInformation::GetTickCount;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};

fn idle_ms() -> windows::core::Result<u32> {
    let mut lii = LASTINPUTINFO { cbSize: size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
    unsafe { GetLastInputInfo(&mut lii).ok()? };
    // wrapping_sub handles the 49.7-day tick wrap and the non-monotonic dwTime note.
    Ok(unsafe { GetTickCount() }.wrapping_sub(lii.dwTime))
}
```

**Hidden message-only window and registrations**

```rust
use windows::core::w;
use windows::Win32::Foundation::{HANDLE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Power::{RegisterPowerSettingNotification, RegisterSuspendResumeNotification};
use windows::Win32::System::RemoteDesktop::{WTSRegisterSessionNotification, NOTIFY_FOR_THIS_SESSION};
use windows::Win32::System::SystemServices::{GUID_SESSION_DISPLAY_STATUS, GUID_SESSION_USER_PRESENCE};
use windows::Win32::UI::WindowsAndMessaging::*;

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_WTSSESSION_CHANGE => {
            match wp.0 as u32 {
                WTS_SESSION_LOCK | WTS_CONSOLE_DISCONNECT | WTS_REMOTE_DISCONNECT => { /* away */ }
                WTS_SESSION_UNLOCK | WTS_CONSOLE_CONNECT | WTS_REMOTE_CONNECT => { /* back */ }
                _ => {}
            }
            LRESULT(0)
        }
        WM_POWERBROADCAST => {
            match wp.0 as u32 {
                PBT_APMSUSPEND => { /* away: machine is sleeping */ }
                PBT_APMRESUMESUSPEND => { /* back: a human woke it */ }
                PBT_POWERSETTINGCHANGE => {
                    let s = &*(lp.0 as *const POWERBROADCAST_SETTING);
                    let value = std::ptr::read_unaligned(s.Data.as_ptr() as *const u32);
                    if s.PowerSetting == GUID_SESSION_DISPLAY_STATUS && value == 0 { /* display off: away */ }
                    if s.PowerSetting == GUID_SESSION_USER_PRESENCE && value == 2 { /* PowerUserInactive */ }
                }
                _ => {}
            }
            LRESULT(1) // "An application should return TRUE if it processes this message."
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

unsafe fn create_listener() -> windows::core::Result<HWND> {
    let hinst = GetModuleHandleW(None)?;
    let class = w!("breakpoint.presence");
    let wc = WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: hinst.into(), lpszClassName: class, ..Default::default() };
    RegisterClassW(&wc);
    let hwnd = CreateWindowExW(WINDOW_EX_STYLE(0), class, w!(""), WINDOW_STYLE(0),
        0, 0, 0, 0, Some(HWND_MESSAGE), None, Some(hinst.into()), None)?;

    WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION)?;           // pair with WTSUnRegisterSessionNotification
    let h = HANDLE(hwnd.0);
    let _susp = RegisterSuspendResumeNotification(h, DEVICE_NOTIFY_WINDOW_HANDLE)?;
    let _disp = RegisterPowerSettingNotification(h, &GUID_SESSION_DISPLAY_STATUS, DEVICE_NOTIFY_WINDOW_HANDLE)?;
    let _pres = RegisterPowerSettingNotification(h, &GUID_SESSION_USER_PRESENCE, DEVICE_NOTIFY_WINDOW_HANDLE)?;
    Ok(hwnd) // keep the HPOWERNOTIFY handles and unregister them on shutdown
}
```

**Screen saver**

```rust
use windows::Win32::Foundation::BOOL;
use windows::Win32::UI::WindowsAndMessaging::{SystemParametersInfoW, SPI_GETSCREENSAVERRUNNING, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS};

fn screensaver_running() -> windows::core::Result<bool> {
    let mut running = BOOL(0);
    unsafe { SystemParametersInfoW(SPI_GETSCREENSAVERRUNNING, 0,
        Some(&mut running as *mut BOOL as *mut _), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0))? };
    Ok(running.as_bool())
}
```

**Full-screen / presentation / locked**

```rust
use windows::Win32::UI::Shell::*;

fn shell_state() -> windows::core::Result<QUERY_USER_NOTIFICATION_STATE> {
    unsafe { SHQueryUserNotificationState() } // QUNS_NOT_PRESENT, QUNS_BUSY, QUNS_RUNNING_D3D_FULL_SCREEN, ...
}
```

**Something is holding the display awake**

```rust
use windows::Win32::System::Power::{CallNtPowerInformation, SystemExecutionState, ES_DISPLAY_REQUIRED};

fn display_required() -> windows::core::Result<bool> {
    let mut state: u32 = 0;
    unsafe { CallNtPowerInformation(SystemExecutionState, None, 0,
        Some(&mut state as *mut u32 as *mut _), size_of::<u32>() as u32).ok()? };
    Ok(state & ES_DISPLAY_REQUIRED.0 != 0)
}
```

**Media playing anywhere (WinRT)**

```rust
use windows::Media::Control::*;

fn media_playing() -> windows::core::Result<bool> {
    let mgr = GlobalSystemMediaTransportControlsSessionManager::RequestAsync()?.get()?;
    for s in mgr.GetSessions()? {
        if s.GetPlaybackInfo()?.PlaybackStatus()? == GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing {
            return Ok(true);
        }
    }
    Ok(false)
}
```

**Presence sensor, if the machine has one (WinRT)**

```rust
use windows::Devices::Sensors::{HumanPresence, HumanPresenceSensor};

fn sensor_says_present() -> Option<bool> {
    let sensor = HumanPresenceSensor::GetDefaultAsync().ok()?.get().ok()?; // Err when no sensor or access denied
    let reading = sensor.GetCurrentReading().ok()?;
    match reading.Presence().ok()? {
        HumanPresence::Present => Some(true),
        HumanPresence::NotPresent => Some(false),
        _ => None,
    }
}
```

**Remote session check and lock-state fallback**

```rust
use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_REMOTESESSION};
let is_rdp = unsafe { GetSystemMetrics(SM_REMOTESESSION) } != 0;
// Lock state without a window: WTSQuerySessionInformationW(None, WTS_CURRENT_SESSION, WTSSessionInfoEx, ..)
// then read (*(buf as *const WTSINFOEXW)).Data.WTSInfoExLevel1.SessionFlags and free with WTSFreeMemory.
```

---

## Recommended design for breakpoint

One `Presence` module owning a tiny state machine, fed by two inputs: a **poll** every 5 s and
**events** from the hidden message-only window. Output is a single enum the nudge scheduler
consumes.

```
enum Presence { Active, IdleAtDesk { since }, Away { since, reason }, Unknown }
```

**Poll (every 5 s, cheap):**

1. `idle_ms()` from `GetLastInputInfo`.
2. If idle_ms < 10 s → `Active`. Done.
3. Otherwise compute the *veto* set: `shell_state()` is `QUNS_BUSY` /
   `QUNS_RUNNING_D3D_FULL_SCREEN` / `QUNS_PRESENTATION_MODE`, or `display_required()`, or
   `media_playing()` (GSMTC; evaluate at most once per poll and skip entirely if the first
   call ever fails). Any veto → `IdleAtDesk` regardless of idle time: the user is present but
   consuming, so the Stand nudge stays pending and is not auto-satisfied.
4. No veto and idle_ms ≥ **90 s** (midpoint of the 60–120 s range; make it a setting) →
   `Away { reason: NoInput }`.
5. If a `HumanPresenceSensor` exists: `NotPresent` promotes to `Away` after 30 s even with a
   media veto (the one case where watching a video and leaving the room are separable);
   `Present` demotes a `NoInput` away back to `IdleAtDesk`.

**Events (push, authoritative, override the poll):**

- `WTS_SESSION_LOCK`, `WTS_CONSOLE_DISCONNECT`, `WTS_REMOTE_DISCONNECT`, `PBT_APMSUSPEND`,
  `GUID_SESSION_DISPLAY_STATUS == PowerMonitorOff`, `GUID_SESSION_USER_PRESENCE ==
  PowerUserInactive`, lid closed → `Away` immediately with that reason.
- `WTS_SESSION_UNLOCK`, `*_CONNECT`, `PBT_APMRESUMESUSPEND`, display `On`,
  `PowerUserPresent` → `Active` and start a fresh sitting block.
- `PBT_APMRESUMEAUTOMATIC` alone → `Unknown` (machine woke itself; wait for input).

**How a nudge is satisfied:** a pending Stand nudge is marked done when `Presence` has been
`Away` for ≥ 60 s cumulative since the nudge fired, whatever the reason. A Drink nudge is
never auto-satisfied (no signal exists). Stop-working is satisfied by `Away` with reason
`Lock`, `Suspend`, `Disconnect` or `DisplayOff` only, not by `NoInput`.

**Timers must survive sleep.** `GetTickCount` keeps counting across sleep but wall-clock
jumps; after any resume, discard in-flight idle measurements and re-arm from the resume event.

**Costs:** one `GetLastInputInfo` + `GetTickCount` every 5 s is negligible; the WinRT and
`CallNtPowerInformation` calls happen only once idle exceeds 10 s. No hooks, no camera, no
polling of other processes.

---

## What this design will get wrong

Honest list, in rough order of how often it will bite:

1. **Standing at the desk.** The user stands up and keeps typing (sit-stand desk). Looks
   `Active`; the Stand nudge is never auto-satisfied. Only the button can clear it.
2. **Watching a windowed video in an app that does not integrate with SMTC and does not set
   `ES_DISPLAY_REQUIRED`.** Looks like `NoInput` away; the nudge self-satisfies while the user
   sits still. The GSMTC and display-required checks reduce this, they do not remove it.
3. **Reading a long document or thinking.** 90 s without input while seated is common. Will be
   mis-read as `Away`. Lengthening the threshold trades this against (4).
4. **Short real breaks.** A 60 s walk to the kettle with a 90 s threshold is missed. Tune the
   threshold per user; do not try to be clever.
5. **Hardware mouse jigglers and other HID emulators.** Indistinguishable from a human; the
   machine looks `Active` forever.
6. **Software jigglers via `SendInput`.** Same result; `GetLastInputInfo` counts them by design.
7. **Audio-only media while away.** A podcast left playing makes `media_playing()` true; if the
   player also holds `ES_DISPLAY_REQUIRED` (some do), the veto keeps the user `IdleAtDesk`
   after they have actually left. `GUID_SESSION_USER_PRESENCE == PowerUserInactive` eventually
   corrects this, on the power plan's timetable, not ours.
8. **Remote Desktop from another room.** The user is upright and walking to a different
   machine, then sitting there; we see `WTS_REMOTE_CONNECT` and input and call them `Active`.
   The nudge is about *their* body, and we have no idea what it is doing.
9. **Another process holding `ES_DISPLAY_REQUIRED`** (a long-running presentation tool, a
   kiosk helper, a badly behaved updater) vetoes idle detection system-wide.
10. **Presence sensor edge cases.** A second person at the desk reads as `Present`; the toggle
    "Let desktop apps access Presence Sensing" off makes the sensor vanish; range is only
    about 1.2 m by the OEM guideline, so a user two metres away but still working on a
    whiteboard reads `NotPresent`.
11. **Start-up race.** `WTSRegisterSessionNotification` can fail with `RPC_S_INVALID_BINDING`
    during early logon; without retry, lock/unlock events are silently missed for the session.
12. **Hibernate-after-sleep.** `PBT_APMRESUMEAUTOMATIC` is not delivered when the machine wakes
    only to hibernate, so a nudge timer may reference a sitting block that spans an overnight
    hibernation. Mitigated by re-arming on `PBT_APMRESUMESUSPEND` / unlock.
13. **Message-only window delivery.** If the plain suspend broadcast turns out not to reach a
    message-only window even after `RegisterSuspendResumeNotification`, breakpoint must fall
    back to the tray's ordinary hidden top-level window. Verify on first implementation.

None of these are fixable without crossing the line into surveillance (camera, keyboard
hooks, process inspection). The design accepts them and keeps the button.

---

## Sources

Win32 references (Microsoft Learn)

- GetLastInputInfo — https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getlastinputinfo
- LASTINPUTINFO — https://learn.microsoft.com/en-us/windows/win32/api/winuser/ns-winuser-lastinputinfo
- GetTickCount — https://learn.microsoft.com/en-us/windows/win32/api/sysinfoapi/nf-sysinfoapi-gettickcount
- SendInput — https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-sendinput
- SetThreadExecutionState — https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-setthreadexecutionstate
- GetSystemMetrics (SM_REMOTESESSION) — https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getsystemmetrics
- WTSRegisterSessionNotification — https://learn.microsoft.com/en-us/windows/win32/api/wtsapi32/nf-wtsapi32-wtsregistersessionnotification
- WM_WTSSESSION_CHANGE — https://learn.microsoft.com/en-us/windows/win32/termserv/wm-wtssession-change
- WTSINFOEXW / WTSINFOEX_LEVEL1_W — https://learn.microsoft.com/en-us/windows/win32/api/wtsapi32/ns-wtsapi32-wtsinfoexw , https://learn.microsoft.com/en-us/windows/win32/api/wtsapi32/ns-wtsapi32-wtsinfoex_level1_w
- WM_POWERBROADCAST — https://learn.microsoft.com/en-us/windows/win32/power/wm-powerbroadcast
- PBT_APMRESUMEAUTOMATIC — https://learn.microsoft.com/en-us/windows/win32/power/pbt-apmresumeautomatic
- RegisterSuspendResumeNotification — https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-registersuspendresumenotification
- RegisterPowerSettingNotification — https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-registerpowersettingnotification
- UnregisterPowerSettingNotification — https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-unregisterpowersettingnotification
- Registering for Power Events — https://learn.microsoft.com/en-us/windows/win32/power/registering-for-power-events
- Power Setting GUIDs — https://learn.microsoft.com/en-us/windows/win32/power/power-setting-guids
- USER_ACTIVITY_PRESENCE — https://learn.microsoft.com/en-us/windows/win32/api/winnt/ne-winnt-user_activity_presence
- POWERBROADCAST_SETTING — https://learn.microsoft.com/en-us/windows/win32/api/winuser/ns-winuser-powerbroadcast_setting
- CallNtPowerInformation — https://learn.microsoft.com/en-us/windows/win32/api/powerbase/nf-powerbase-callntpowerinformation
- Window Features (message-only windows) — https://learn.microsoft.com/en-us/windows/win32/winmsg/window-features
- CreateWindowExW — https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-createwindowexw
- SystemParametersInfoW — https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-systemparametersinfow
- SHQueryUserNotificationState — https://learn.microsoft.com/en-us/windows/win32/api/shellapi/nf-shellapi-shqueryusernotificationstate
- QUERY_USER_NOTIFICATION_STATE — https://learn.microsoft.com/en-us/windows/win32/api/shellapi/ne-shellapi-query_user_notification_state

WinRT references (Microsoft Learn)

- HumanPresenceSensor — https://learn.microsoft.com/en-us/uwp/api/windows.devices.sensors.humanpresencesensor
- HumanPresenceSensor.GetDefault / GetDefaultAsync — https://learn.microsoft.com/en-us/uwp/api/windows.devices.sensors.humanpresencesensor.getdefault , https://learn.microsoft.com/en-us/uwp/api/windows.devices.sensors.humanpresencesensor.getdefaultasync
- HumanPresenceSensorReading — https://learn.microsoft.com/en-us/uwp/api/windows.devices.sensors.humanpresencesensorreading
- HumanPresence enum — https://learn.microsoft.com/en-us/uwp/api/windows.devices.sensors.humanpresence
- GlobalSystemMediaTransportControlsSessionManager — https://learn.microsoft.com/en-us/uwp/api/windows.media.control.globalsystemmediatransportcontrolssessionmanager
- GlobalSystemMediaTransportControlsSessionPlaybackInfo — https://learn.microsoft.com/en-us/uwp/api/windows.media.control.globalsystemmediatransportcontrolssessionplaybackinfo
- GlobalSystemMediaTransportControlsSessionPlaybackStatus — https://learn.microsoft.com/en-us/uwp/api/windows.media.control.globalsystemmediatransportcontrolssessionplaybackstatus
- App capability declarations (globalMediaControl, humanPresence, which apps capabilities apply to) — https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/app-capability-declarations

Presence-sensing hardware and settings (Microsoft)

- Managing Presence Sensing settings in Windows 11 — https://support.microsoft.com/en-US/Windows/privacy/managing-presence-sensing-settings-in-windows-11
- Presence sensors (OEM component guidelines) — https://learn.microsoft.com/en-us/windows-hardware/design/component-guidelines/sensors-presence-sensors

`windows` crate

- Crate page (0.62.2, dependencies) — https://docs.rs/crate/windows/latest
- Feature flags for 0.62.2 — https://docs.rs/crate/windows/latest/features
- API docs root (0.62.2) — https://microsoft.github.io/windows-docs-rs/doc/windows/
- Readme on main (future 0.100 feature naming) — https://github.com/microsoft/windows-rs/blob/master/crates/libs/windows/readme.md
- Readme at 0.62.0 (namespace-style features) — https://github.com/microsoft/windows-rs/blob/0.62.0/crates/libs/windows/readme.md
- IAsyncOperation::get — https://docs.rs/windows-future/0.2.1/windows_future/struct.IAsyncOperation.html
