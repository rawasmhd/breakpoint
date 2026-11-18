//! The Windows-only layer: tray, presence inputs, popup, dialogs, startup, clock and log files.

pub mod clock;
pub mod dialogs;
pub mod flyout;
// The form kit. Part of its API has no caller yet: ghost buttons, `set_text` and `fit`, and
// `focused`, which UI Automation (#80) will use.
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
