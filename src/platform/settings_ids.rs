// The settings window's and the first-run wizard's resource and control ids, in one place for
// both sides: `build.rs` includes this file to write the DIALOGEX templates, and `dialogs.rs`
// includes it to find the controls. Plain constants only, so the file compiles in either crate.

/// The settings window's DIALOGEX template.
pub const IDD_SETTINGS: u16 = 101;
/// The first-run wizard's DIALOGEX template.
pub const IDD_WIZARD: u16 = 102;

/// The seven work-day checkboxes, Sunday first: `IDC_DAY` to `IDC_DAY + 6`.
pub const IDC_DAY: i32 = 1000;
pub const IDC_WEEK_HEADING: i32 = 1007;
pub const IDC_DAYS_LABEL: i32 = 1008;
pub const IDC_FROM: i32 = 1010;
pub const IDC_TO: i32 = 1011;
pub const IDC_HOURS_LABEL: i32 = 1012;
/// The "to" between the two time pickers.
pub const IDC_HOURS_TO: i32 = 1013;
pub const IDC_PROFILE: i32 = 1020;
pub const IDC_NUDGES_HEADING: i32 = 1021;
pub const IDC_PROFILE_LABEL: i32 = 1022;

pub const IDC_STAND_ON: i32 = 1030;
pub const IDC_STAND_EVERY: i32 = 1031;
pub const IDC_STAND_EVERY_SPIN: i32 = 1032;
pub const IDC_STAND_LENGTH: i32 = 1033;
pub const IDC_STAND_LENGTH_SPIN: i32 = 1034;
pub const IDC_STAND_LEVEL: i32 = 1035;
pub const IDC_STAND_EVERY_LABEL: i32 = 1036;
pub const IDC_STAND_EVERY_UNIT: i32 = 1037;
pub const IDC_STAND_LENGTH_LABEL: i32 = 1038;
pub const IDC_STAND_LENGTH_UNIT: i32 = 1039;

pub const IDC_PAUSE_ON: i32 = 1040;
pub const IDC_PAUSE_EVERY: i32 = 1041;
pub const IDC_PAUSE_EVERY_SPIN: i32 = 1042;
pub const IDC_PAUSE_LENGTH: i32 = 1043;
pub const IDC_PAUSE_LENGTH_SPIN: i32 = 1044;
pub const IDC_PAUSE_LEVEL: i32 = 1045;
pub const IDC_PAUSE_EVERY_LABEL: i32 = 1046;
pub const IDC_PAUSE_EVERY_UNIT: i32 = 1047;
pub const IDC_PAUSE_LENGTH_LABEL: i32 = 1048;
pub const IDC_PAUSE_LENGTH_UNIT: i32 = 1049;

pub const IDC_WATER_ON: i32 = 1050;
pub const IDC_WATER_EVERY: i32 = 1051;
pub const IDC_WATER_EVERY_SPIN: i32 = 1052;
pub const IDC_WATER_EVERY_LABEL: i32 = 1053;
pub const IDC_WATER_EVERY_UNIT: i32 = 1054;
/// "Always gentle; rides along with the next nudge."
pub const IDC_WATER_NOTE: i32 = 1055;

pub const IDC_START_WITH_WINDOWS: i32 = 1060;

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
