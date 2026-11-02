---
status: accepted
date: 2026-11-02
---

# Rust for the desktop app

breakpoint is a Windows tray app whose whole value is being invisible and tiny, and the obvious stack for a Windows tray app is C# with WinForms. We chose **Rust** instead (with `tray-icon` and the `windows` crate) because the end-user footprint matters more to us than development speed: one exe of a few megabytes, no runtime to install, instant start, a few megabytes of RAM. C# would have been faster to write but needs the .NET Desktop Runtime or a 30 to 70 MB self-contained build.

## Considered options

- **C# / .NET WinForms**: best developer speed and Windows integration; rejected for the runtime dependency or the self-contained size.
- **Python + pystray**: already installed; rejected for 15 to 30 MB PyInstaller exes, slow start and antivirus false positives.
- **Go, AutoHotkey, Tauri, Electron**: rejected for a hacky toast story, poor contributor experience, or sheer size.

## Consequences

Windows UI (settings window, wizard, any popup) is more work in Rust than in C#; the choice of UI technology is its own research ticket. The scheduler core must stay free of the `windows` crate so it compiles and tests anywhere.
