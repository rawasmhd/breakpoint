# breakpoint: product and technical spec

Status: draft v0.2, 2026-11-02. Companion to [RESEARCH.md](RESEARCH.md) (the why) and [BACKLOG.md](BACKLOG.md) (the tickets).

## 1. One-sentence pitch

**Breakpoints for humans.** A tiny Windows tray app that quietly reminds you to rest your eyes, stand up, drink water, eat lunch and stop working, with evidence-informed defaults, zero required configuration, and no screen takeovers.

## 2. Goals and non-goals

**Goals**
1. Works within 10 seconds of launching: sensible defaults, nothing to set up.
2. Invisible until needed: lives in the tray, uses native Windows toasts.
3. Respects context: silent when you are idle, locked, presenting or outside work hours.
4. Lean: a single executable of a few megabytes, low memory, no background services, no telemetry, no runtime to install.
5. Hackable: one JSON config file, open source, easy to contribute to.

**Non-goals (v1)**
- No mandatory full-screen break overlays (Stretchly/Workrave style). An optional overlay for "stop working" is a later ticket.
- No time tracking, task management, Pomodoro task lists or cloud sync.
- No macOS/Linux in v1. Windows 10 (1809+) and Windows 11 only. The core is OS-agnostic so ports stay possible.
- No telemetry or accounts.

## 3. Users

- **Primary:** a knowledge worker on Windows who wants health nudges without fiddling. Installs, forgets, benefits.
- **Secondary:** a tinkerer who wants to change intervals, add custom reminders and read the code.

## 4. Reminder model

| Kind | Fires | Examples | Clock behaviour |
|---|---|---|---|
| **Interval** | every N minutes of *active* time | eyes, stand, hydrate, long break | pauses on idle/lock/outside hours; resets after a long idle |
| **Clock** | at a wall-clock time on scheduled days | lunch, stop working | fires once per day; skipped if missed by more than 30 min |

Each reminder has: `id`, `title`, `message`, `enabled`, `intervalMinutes` or `at`, optional `durationSeconds` (shows a countdown), optional `warnMinutesBefore` (clock only), and `mergeWith` (fold into another reminder when both fall within 2 minutes, to avoid double toasts).

### Default set (Balanced profile)

```json
{
  "profile": "balanced",
  "workHours": { "start": "09:00", "end": "18:00", "days": ["Mon", "Tue", "Wed", "Thu", "Fri"] },
  "idleThresholdMinutes": 5,
  "reminders": [
    { "id": "eyes",  "title": "Rest your eyes",    "message": "Look at something 20 feet away for 20 seconds.",   "intervalMinutes": 20, "durationSeconds": 20 },
    { "id": "stand", "title": "Stand up",          "message": "Stand, stretch, walk for a minute or two.",        "intervalMinutes": 30, "durationSeconds": 120 },
    { "id": "water", "title": "Drink water",       "message": "Have a glass of water.",                           "intervalMinutes": 60, "mergeWith": "stand" },
    { "id": "break", "title": "Take a real break", "message": "Step away from the screen for 5 to 10 minutes.",   "intervalMinutes": 90, "durationSeconds": 300 },
    { "id": "lunch", "title": "Lunch",             "message": "Time to eat. Away from the desk if you can.",      "at": "12:30" },
    { "id": "stop",  "title": "Stop working",      "message": "You have done your hours. Wrap up and log off.",   "at": "18:00", "warnMinutesBefore": 15 }
  ]
}
```

Profiles only change numbers: **Gentle** (eyes 30, stand 45, break 120), **Balanced** (above), **Strict** (eyes 20, stand 25, break 60, and stop-working nudges repeat every 10 min until you confirm or pause).

## 5. Behaviour

- **Toast content:** title, message, and two buttons: **Done** and **Snooze 5 min**.
- **Snooze:** delays that one reminder by 5 minutes (configurable). Max 3 snoozes per occurrence, then it is skipped and logged as skipped.
- **Pause (tray menu):** 30 min, 1 hour, 2 hours, until tomorrow, or resume. Tray icon changes to a paused glyph.
- **Idle:** no keyboard/mouse input for `idleThresholdMinutes` pauses interval timers. Idle for 15+ minutes counts as a break taken: eyes/stand/water timers reset on return.
- **Lock / sleep:** treated as idle. On resume from sleep, timers reset.
- **Quiet contexts:** no toasts while a full-screen app is running or presentation mode is on (Windows user notification state). The reminder is deferred up to 10 minutes, then skipped.
- **Work hours:** outside the window no interval reminders fire. Clock reminders fire only on scheduled days.
- **Stop working escalation (Strict only):** after the 18:00 toast, repeat every 10 minutes until the user clicks "Done for today" or pauses.
- **Start with Windows:** toggle in the tray menu, via the per-user Run registry key. Offered once on first run.
- **Single instance:** a second launch flashes the existing tray icon and exits.
- **Config:** `%APPDATA%\breakpoint\config.json`. Written with defaults on first run, watched and hot-reloaded. "Edit settings" in the tray opens it in the default editor. Common settings (profile, enable/disable each reminder, work-hours presets) are also directly in the tray menu, so most people never open the file.
- **Log:** `%APPDATA%\breakpoint\log.jsonl`, rotated at 1 MB. Records fired/snoozed/skipped events so a daily summary is possible.

## 6. Technical stack decision

### Options considered

| Option | Exe size | Runtime needed | Dev speed | Windows integration | Contributors | Verdict |
|---|---|---|---|---|---|---|
| **Rust + `tray-icon` + WinRT toasts via the `windows` crate** | 1 to 3 MB | none | medium | excellent: every Win32/WinRT API is a direct call | medium, growing | **chosen** |
| C# / .NET 10 WinForms tray | ~1 MB plus the .NET Desktop Runtime, or 30 to 70 MB self-contained | .NET runtime unless self-contained | high | excellent | high | closest runner-up; rejected for the runtime dependency |
| Python 3 + pystray + winotify, PyInstaller | 15 to 30 MB | none bundled | high | adequate | medium | antivirus false positives on PyInstaller exes, slow start |
| Go + systray + go-toast | ~10 MB | none | medium | toasts via PowerShell shim | medium | hacky toast story |
| AutoHotkey v2 compiled | ~1 MB | none | very high | tray and TrayTip built in | low | worst for contributors and tests |
| Tauri 2 / Electron | 5 MB / 150 MB | WebView2 / Chromium | medium | via plugins | high | overkill for an app with almost no UI |

### Decision

**Rust, stable toolchain.** Smallest possible footprint for end users: one exe of a few megabytes, no runtime, instant start, a few MB of RAM. The cost is slower development than C#, accepted deliberately.

Crates, kept to a minimum:

| Concern | Crate | Notes |
|---|---|---|
| Tray icon and menu | `tray-icon` + `muda` | From the Tauri team, actively maintained. Needs a Win32 message loop, which we run ourselves with a hidden message-only window (no `winit`). |
| Win32 / WinRT calls | `windows` | `GetLastInputInfo` (idle), `SHQueryUserNotificationState` (fullscreen/presenting), `WTSRegisterSessionNotification` (lock), `WM_POWERBROADCAST` (sleep), `CreateMutexW` (single instance), registry Run key, and `Windows.UI.Notifications` for toasts with buttons |
| Config | `serde` + `serde_json` | Hot reload by polling the file's modified time every 2 s. No file-watcher crate. |
| Time | `jiff` | Local wall-clock time, weekdays, parsing "HH:MM" |
| Exe icon and version info | `winresource` (build dependency) | |
| Tests | built-in `cargo test` | Scheduler tested with an injected clock |

Toasts: phase one uses the PowerShell AppUserModelID so toasts work with zero registration (Windows shows them under a generic app name). Phase two registers our own AUMID through a Start Menu shortcut so the toast carries the breakpoint name and icon. Button clicks are handled through the toast's in-process `Activated` event while the app runs, which is always, since it is a tray app.

Release profile: `opt-level = "z"`, `lto = true`, `codegen-units = 1`, `panic = "abort"`, `strip = true`, and `#![windows_subsystem = "windows"]` so no console window appears.

Targets: the `windows` crate and `tray-icon` build on both `x86_64-pc-windows-msvc` and `x86_64-pc-windows-gnu`. Release binaries come from CI on the MSVC target (smaller, no MinGW runtime concerns). The dev machine currently has Rust 1.99 with the GNU toolchain only, which is fine for local development. Ticket 1 decides whether to also install the MSVC linker locally (Visual Studio Build Tools, "Desktop development with C++" workload) so local and CI builds match.

### Architecture

```
src/
  main.rs              single-instance guard, message loop, wiring
  core/
    mod.rs
    scheduler.rs       pure logic: config + now + activity state -> due reminders, next-due time
    reminder.rs        model, profiles, defaults
    config.rs          load/save, defaults, validation
  platform/
    tray.rs            tray-icon + muda menu
    idle.rs            GetLastInputInfo, session lock, power events
    quiet.rs           SHQueryUserNotificationState
    startup.rs         Run key
    toast.rs           WinRT toast with Done / Snooze buttons
  log.rs               JSON-lines event log with rotation
tests/
  scheduler.rs         integration-style tests with a fake clock
```

One rule: `core/` has no `windows` crate dependency, so scheduling logic compiles and tests on any OS and a future macOS or Linux port only needs a new `platform/`.

## 7. Distribution

- GitHub Releases: `breakpoint.exe`, built by GitHub Actions on every `v*` tag, plus a SHA-256 checksum.
- Later: winget manifest so `winget install breakpoint` works, and a Scoop manifest.
- License: MIT.

## 8. Open questions

1. First-run: prompt for work hours, or use 09:00 to 18:00 and let the user edit? (Proposed: defaults plus a one-time welcome toast pointing at the tray menu.)
2. "Stop working" escalation on in Balanced, or Strict only? (Proposed: Strict only.)
3. Code signing: unsigned exes trigger SmartScreen on first run. Options are documenting the "More info, Run anyway" step, Azure Trusted Signing (cheap, needs a verified identity), or building reputation over time. (Proposed: document first, revisit after release 1.0.)
