// The first-run wizard's resource and control ids, in one place for both sides: `build.rs`
// includes this file to write the DIALOGEX template, and `dialogs.rs` includes it to find the
// controls. Plain constants only, so the file compiles in either crate. The settings window is
// a painted kit window with its own item ids (`settings_window.rs`).

/// The first-run wizard's DIALOGEX template.
pub const IDD_WIZARD: u16 = 102;

// The wizard's controls.

/// The three profile radio buttons in `Profile::ALL` order: `IDC_WIZARD_PROFILE` to
/// `IDC_WIZARD_PROFILE + 2`.
pub const IDC_WIZARD_PROFILE: i32 = 1100;
/// The numbers line under each profile radio button, in the same order: `IDC_WIZARD_NUMBERS`
/// to `IDC_WIZARD_NUMBERS + 2`.
pub const IDC_WIZARD_NUMBERS: i32 = 1103;
pub const IDC_WIZARD_START_WITH_WINDOWS: i32 = 1110;
/// The read-only work-week line.
pub const IDC_WIZARD_WEEK: i32 = 1111;
pub const IDC_WIZARD_CHANGE: i32 = 1112;
/// "Pick a rhythm".
pub const IDC_WIZARD_TITLE: i32 = 1113;
pub const IDC_WIZARD_PROFILE_HEADING: i32 = 1114;
pub const IDC_WIZARD_WEEK_HEADING: i32 = 1115;
/// "breakpoint lives in the tray, …".
pub const IDC_WIZARD_TRAY_NOTE: i32 = 1116;
