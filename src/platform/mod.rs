//! The Windows-only layer: tray, presence inputs, popup, dialogs, startup, clock and log files.

pub mod clock;
pub mod dialogs;
// The form kit is adopted by the settings window (#78) and the wizard (#79).
#[allow(dead_code)]
pub mod kit;
pub mod log;
pub mod paint;
pub mod popup;
pub mod presence;
pub mod startup;
pub mod theme;
pub mod tray;
