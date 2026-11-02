# Backlog

Mirrored into GitHub Issues. Milestones: **M0 Setup**, **M1 MVP**, **M2 Polish**, **M3 Distribution**.
Each ticket lists acceptance criteria (AC). Sizes are S / M / L.

## M0: Project setup

### 1. Scaffold the Rust project (S)
- `cargo init` as a binary crate named `breakpoint`, edition 2024, `rust-toolchain.toml` pinning stable.
- `#![windows_subsystem = "windows"]`, release profile tuned for size (`opt-level = "z"`, `lto`, `codegen-units = 1`, `panic = "abort"`, `strip`).
- `rustfmt.toml`, `clippy.toml`, `.editorconfig`, `.gitignore`.
- The dev machine has the `x86_64-pc-windows-gnu` toolchain only. Confirm `tray-icon` and `windows` build on it, and decide whether to install the MSVC linker locally (Visual Studio Build Tools, C++ workload) so local builds match the CI release target. Document the choice in CONTRIBUTING.
- AC: `cargo build --release` produces `breakpoint.exe` under 1 MB for a hello-world tray-less binary, `cargo clippy -- -D warnings` and `cargo fmt --check` pass.

### 2. CI: build, test and release pipeline (S)
- GitHub Actions on `windows-latest`: fmt check, clippy, `cargo test`, release build, on push and PR. Cache cargo registry and target.
- On tag `v*`: build release exe, compute SHA-256, create a GitHub Release with auto-generated notes and both files attached.
- AC: a tagged push produces a Release with `breakpoint.exe` and `breakpoint.exe.sha256`.

## M1: MVP

### 3. Tray app shell (M)
- Hidden message-only window and Win32 message loop. `tray-icon` with icon and tooltip showing the next reminder. `muda` context menu: Pause (30m / 1h / 2h / until tomorrow), Resume, Profile (Gentle / Balanced / Strict), Reminders (checkbox per reminder), Edit settings file, Start with Windows (checkbox), About, Quit.
- Single-instance guard with a named mutex; a second launch exits.
- AC: app starts to tray in under 300 ms, every menu item works, Quit removes the icon cleanly, idle memory under 10 MB.

### 4. Scheduler core (M)
- Pure `Scheduler` in `core/`: input is config, current time, activity state (active / idle / locked / quiet); output is reminders due now and next-due time for the tooltip.
- Interval reminders count active time only. Clock reminders fire once per day, skipped if missed by more than 30 min.
- `mergeWith` folds a due reminder into its partner when both fall within 2 minutes.
- AC: tests cover interval, clock, merge, snooze cap, work-hours boundary, idle reset (see ticket 12). No `windows` crate import inside `core/`.

### 5. Default reminder set and profiles (S)
- Balanced defaults from SPEC section 4. Gentle and Strict profiles as numeric overrides.
- AC: fresh install with no config yields the Balanced set; switching profile from the tray menu rewrites config and changes intervals immediately.

### 6. Notifications phase 1: WinRT toasts (M)
- `Toaster` trait. Windows implementation uses `Windows.UI.Notifications` via the `windows` crate with the PowerShell AUMID so no registration is needed. Title, message, countdown text for reminders with a duration.
- AC: each due reminder shows one native toast; no toast while paused; works on a clean Windows 11 VM.

### 7. JSON config: load, defaults, hot reload (M)
- `%APPDATA%\breakpoint\config.json`, created with defaults on first run. Poll modified time every 2 s and reload on change. Invalid JSON: keep last good config and show one error toast.
- AC: editing an interval in the file changes the next fire time without restart; a broken file does not crash the app.

### 8. Idle, lock and sleep awareness (M)
- `GetLastInputInfo` polled every 10 s. Idle beyond `idleThresholdMinutes` pauses interval timers. Idle 15+ min resets eyes/stand/water on return.
- `WTSRegisterSessionNotification` for lock/unlock and `WM_POWERBROADCAST` for sleep/resume on the hidden window, treated as idle start/end; resume resets timers.
- AC: lock the PC for 20 min, unlock: no backlog of toasts, timers restart from zero.

### 9. Work hours and weekdays (S)
- Interval reminders only inside `workHours` on listed days. Clock reminders only on listed days.
- AC: at 18:01 no interval toasts; on Saturday nothing fires.

### 10. Snooze and pause (S)
- Snooze delays one occurrence 5 min, max 3 times, then logs skipped. Pause durations from tray menu; icon switches to paused glyph; tooltip shows resume time.
- AC: pausing for 30 min suppresses everything and resumes automatically.

### 11. Start with Windows (S)
- Toggle writes/removes `HKCU\Software\Microsoft\Windows\CurrentVersion\Run\breakpoint`. Offered once on first run via a toast.
- AC: the checkbox reflects actual registry state every time the menu opens.

### 12. Tests for scheduler and config (M)
- Injected clock; tests for every rule in tickets 4, 5, 9, 10. Property test for "no reminder fires twice within its interval".
- AC: `cargo test` runs in CI; every public function in `core/` has at least one test.

### 13. Event log (S)
- Append fired / snoozed / skipped / paused events to `%APPDATA%\breakpoint\log.jsonl`, rotate at 1 MB.
- AC: one JSON object per line with timestamp, reminder id, event.

## M2: Polish

### 14. Quiet mode: fullscreen and presentation (S)
- `SHQueryUserNotificationState`: when busy, defer up to 10 min then skip. Honour Focus Assist / Do Not Disturb where detectable.
- AC: toasts never appear during a full-screen video or a slideshow.

### 15. Notifications phase 2: own identity and Done / Snooze buttons (M)
- Register a breakpoint AUMID via a Start Menu shortcut so toasts show the app name and icon. Add Done and Snooze 5 min buttons, handled through the in-process `Activated` event.
- AC: toast header says "breakpoint" with its icon; Snooze delays the reminder; Done dismisses and logs.

### 16. Tray-menu settings and work-hours presets (S)
- Work hours submenu with presets (8 to 17, 9 to 18, 10 to 19, custom in file). Idle threshold submenu. Everything writes the same JSON.
- AC: every tray setting round-trips to config.json; the file stays the single source of truth.

### 17. Stop-working escalation and optional overlay (M)
- Strict profile repeats the stop toast every 10 min. Optional dimmed full-screen overlay window with "Done for today" and "10 more minutes" buttons.
- AC: overlay is off by default, closable, never traps input.

### 18. Sounds (S)
- Optional short sound per reminder, default off, WAV path from config or built-in, via `PlaySoundW`.
- AC: sound plays only when enabled and respects quiet mode.

### 19. Daily summary (S)
- Tray menu "Today": reminders taken vs skipped from the log. Optional end-of-day summary toast.
- AC: counts match log entries.

## M3: Distribution

### 20. Release hardening (S)
- App icon and version info via `winresource`, panic hook that writes to the log, document SmartScreen "Run anyway" for unsigned builds, evaluate Azure Trusted Signing.
- AC: Release exe runs on a clean Windows 11 VM with no Rust installed.

### 21. winget and Scoop manifests (S)
- Submit to winget-pkgs and a Scoop bucket after the first stable release.
- AC: `winget install breakpoint` works.

### 22. Docs: user guide and contributing (S)
- README install and usage, config reference, CONTRIBUTING.md, issue templates.
- AC: a new contributor can build and run from README alone.
