# breakpoint

**Breakpoints for humans.** A tiny Windows tray app that nudges you to stand and move, drink water and pause between focus periods. It notices when you actually step away, so a nudge you already obeyed goes away by itself. One small exe, no installer, no telemetry.

**Status:** spec locked, nothing to run yet. The [v1 spec](docs/SPEC.md) was settled on the [wayfinder map](https://github.com/rawasmhd/breakpoint/issues/23); build issues come next. The [research behind the defaults](docs/RESEARCH.md) and the [glossary](GLOSSARY.md) are current.

## Why

Break-reminder apps exist, but most are heavyweight, take over your screen, or make you click "Done" for a break you already took. breakpoint aims to be:

- **Behaviour-aware.** Step away from the computer and the nudge counts as done. Lock the screen and it counts at once. There is no Done button.
- **Quiet.** A tray icon and a small popup above the tray that never steals your keyboard. Silent outside your work hours and on your weekend, and one click turns it off until the next work day.
- **Honest about evidence.** Defaults come from the best available research, graded in [RESEARCH.md](docs/RESEARCH.md), including where the evidence is thin.
- **Lean.** A single Rust executable, one JSON settings file, no runtime, no services, no telemetry.

## Default schedule (Balanced profile)

| Nudge | When | Satisfied by |
|---|---|---|
| Stand/Move | every 30 min of active time | being away for 2 min, or locking the screen |
| Pause | every 60 min of active time (it insists) | being away for 5 min, or locking the screen |
| Water | every 60 min, as a line on the next nudge | nothing: it is a heads-up |

Gentle (45/90) and Strict (30/30, a Pomodoro) are picked in the first-run page or in Settings, and every number stays editable. Default work week: Sunday to Thursday, 08:00 to 17:00, changeable in Settings.

## Tech

Rust on the MSVC toolchain, `tray-icon` for the tray, the `windows` crate for everything else (raw Win32 dialogs and popup, presence detection). Why Rust: [ADR 0001](docs/adr/0001-rust-for-the-desktop-app.md). Design and crates: [SPEC.md, technical design](docs/SPEC.md#12-technical-design).

## Files

Settings live in `%APPDATA%\breakpoint\config.json` and a local event log in `%APPDATA%\breakpoint\log\`. To uninstall: untick Start with Windows in Settings, quit from the tray, then delete `breakpoint.exe` and that folder.

## Contributing

Build issues will be created from the [spec](docs/SPEC.md). You need stable Rust and the MSVC build tools (Visual Studio Build Tools, "Desktop development with C++").

```
cargo build --release
cargo test
```

## License

MIT
