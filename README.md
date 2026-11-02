# breakpoint

**Breakpoints for humans.** A tiny Windows tray app that reminds you to rest your eyes, stand up, drink water, have lunch and stop working. Evidence-informed defaults, zero setup, native toasts, no screen takeovers, one small exe.

**Status:** planning. Nothing to run yet. See the [spec](docs/SPEC.md), the [research behind the defaults](docs/RESEARCH.md) and the [backlog](docs/BACKLOG.md). Issues are the live task list.

## Why

Break-reminder apps exist, but most are either heavyweight (Electron), take over your screen, or stop at eye breaks. breakpoint aims to be:

- **Quiet.** A tray icon and normal Windows notifications. Silent when you are idle, locked, presenting or off the clock.
- **Sensible out of the box.** 20-20-20 eye breaks, stand every 30 minutes, a longer break every 90, lunch, and a firm "stop working" nudge at the end of your day. Defaults come from the best available evidence, graded honestly in [RESEARCH.md](docs/RESEARCH.md).
- **Lean.** A single Rust executable of a few megabytes, one JSON config file, no runtime, no services, no telemetry.

## Default schedule (Balanced profile)

| Reminder | When |
|---|---|
| Rest your eyes | every 20 min of active time, 20 s |
| Stand up | every 30 min, 2 min |
| Drink water | every 60 min, folded into the stand reminder |
| Take a real break | every 90 min, 5 to 10 min |
| Lunch | 12:30 |
| Stop working | 18:00, warning at 17:45 |

Gentle and Strict profiles are one click away in the tray menu. Everything is editable in `%APPDATA%\breakpoint\config.json`.

## Tech

Rust, `tray-icon` for the tray, the `windows` crate for toasts and system state. Alternatives considered and the reasoning are in [SPEC.md](docs/SPEC.md#6-technical-stack-decision).

## Contributing

Issues are the backlog. Pick one, comment, open a PR. You need stable Rust and the MSVC build tools.

```
cargo build --release
cargo test
```

## License

MIT
