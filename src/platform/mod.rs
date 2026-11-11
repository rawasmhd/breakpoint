//! The Windows-only layer: tray, presence inputs, popup, dialogs, startup, clock and log files.

pub mod clock;
pub mod dialogs;
// The form kit: the settings window uses it; the wizard (#79) will use the rest.
#[allow(dead_code)]
pub mod kit;
pub mod log;
pub mod paint;
pub mod popup;
pub mod presence;
pub mod settings_window;
pub mod startup;
pub mod theme;
pub mod tray;
