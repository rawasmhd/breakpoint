//! Embeds the exe icons, version info, application manifest and the dialog templates.

#[allow(dead_code)]
mod ids {
    include!("src/platform/settings_ids.rs");
}

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    println!("cargo:rerun-if-changed=assets");
    println!("cargo:rerun-if-changed=src/platform/settings_ids.rs");

    let mut res = winresource::WindowsResource::new();
    // Icon 1 is the exe icon and the tray's On state; icon 2 is Quiet, icon 3 is Pending.
    res.set_icon_with_id("assets/breakpoint.ico", "1");
    res.set_icon_with_id("assets/breakpoint-grey.ico", "2");
    res.set_icon_with_id("assets/breakpoint-pending.ico", "3");
    res.set_manifest_file("assets/breakpoint.exe.manifest");
    // The SDK headers give the template its style names.
    res.add_toolkit_include(true);
    res.append_rc_content(&settings_dialog());
    res.append_rc_content(&wizard_dialog());
    res.compile().expect("failed to embed Windows resources");
}

/// Every control is at 0, 0, 0, 0 here: `dialogs.rs` places each one in pixels from the design
/// tokens, for the monitor's DPI, and sets the fonts. So the templates give no font, and their
/// order is the tab order. Statics that `dialogs.rs` places have ids; LTEXT starts a new group.
const PLACED: &str = "0, 0, 0, 0";

/// The first-run wizard (SPEC §9.1): as wide as the settings window, which Change… opens at the
/// same position. The profile names and lines, the work-week line and "Change…" are set at run
/// time: the core writes the lines, and the resource compiler would mangle the ellipsis.
fn wizard_dialog() -> String {
    use ids::*;
    // The radio buttons sit together, so the arrow keys move only among them; the first starts
    // the group and is the tab stop. The numbers lines follow them.
    let radios = (0..3)
        .map(|i| {
            let first = if i == 0 {
                " | WS_GROUP | WS_TABSTOP"
            } else {
                ""
            };
            format!(
                "    AUTORADIOBUTTON \"\", {}, {PLACED}, BS_TOP{first}\n",
                IDC_WIZARD_PROFILE + i
            )
        })
        .collect::<String>();
    let numbers = (0..3)
        .map(|i| {
            format!(
                "    LTEXT \"\", {}, {PLACED}, SS_NOTIFY\n",
                IDC_WIZARD_NUMBERS + i
            )
        })
        .collect::<String>();
    format!(
        r#"
{IDD_WIZARD} DIALOGEX 0, 0, 300, 200
STYLE DS_MODALFRAME | DS_CENTER | WS_POPUP | WS_CAPTION | WS_SYSMENU
EXSTYLE WS_EX_APPWINDOW
CAPTION "breakpoint"
BEGIN
    LTEXT "Pick a rhythm", {IDC_WIZARD_TITLE}, {PLACED}
    LTEXT "Profile", {IDC_WIZARD_PROFILE_HEADING}, {PLACED}
{radios}{numbers}    LTEXT "Work week", {IDC_WIZARD_WEEK_HEADING}, {PLACED}
    LTEXT "", {IDC_WIZARD_WEEK}, {PLACED}
    PUSHBUTTON "", {IDC_WIZARD_CHANGE}, {PLACED}, WS_GROUP | WS_TABSTOP
    AUTOCHECKBOX "Start with Windows", {IDC_WIZARD_START_WITH_WINDOWS}, {PLACED}, WS_GROUP | WS_TABSTOP
    LTEXT "breakpoint lives in the tray, under ^ if you don't see it.", {IDC_WIZARD_TRAY_NOTE}, {PLACED}
    CONTROL "Start", IDOK, "Button", BS_OWNERDRAW | WS_GROUP | WS_TABSTOP, {PLACED}
END
"#
    )
}

/// The settings window (SPEC §9.2): one fixed-size page. Tab order is the order below; each
/// up-down control follows its edit box, which it takes as its buddy.
fn settings_dialog() -> String {
    use ids::*;
    let days = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"]
        .iter()
        .enumerate()
        .map(|(i, day)| {
            format!(
                "    AUTOCHECKBOX \"{day}\", {}, {PLACED}\n",
                IDC_DAY + i as i32
            )
        })
        .collect::<String>();
    let spin = |id: i32| {
        format!(
            "    CONTROL \"\", {id}, \"msctls_updown32\", UDS_AUTOBUDDY | UDS_SETBUDDYINT | \
             UDS_ALIGNRIGHT | UDS_ARROWKEYS | UDS_NOTHOUSANDS, {PLACED}\n"
        )
    };
    // One nudge row: on, "every" [n] "min", then for Stand/Move and Pause, "<x> length" [n] "min"
    // and the level.
    let row = |name: &str, [on, every, every_spin, every_label, every_unit]: [i32; 5]| {
        format!(
            "    AUTOCHECKBOX \"{name}\", {on}, {PLACED}\n\
             \x20   LTEXT \"every\", {every_label}, {PLACED}\n\
             \x20   EDITTEXT {every}, {PLACED}, ES_NUMBER\n\
             {}\
             \x20   LTEXT \"min\", {every_unit}, {PLACED}\n",
            spin(every_spin),
        )
    };
    let length = |length: &str, [len, len_spin, len_label, len_unit, level]: [i32; 5]| {
        format!(
            "    LTEXT \"{length} length\", {len_label}, {PLACED}\n\
             \x20   EDITTEXT {len}, {PLACED}, ES_NUMBER\n\
             {}\
             \x20   LTEXT \"min\", {len_unit}, {PLACED}\n\
             \x20   COMBOBOX {level}, {PLACED}, CBS_DROPDOWNLIST | WS_VSCROLL | WS_TABSTOP\n",
            spin(len_spin),
        )
    };
    let stand = row(
        "Stand/Move",
        [
            IDC_STAND_ON,
            IDC_STAND_EVERY,
            IDC_STAND_EVERY_SPIN,
            IDC_STAND_EVERY_LABEL,
            IDC_STAND_EVERY_UNIT,
        ],
    ) + &length(
        "stand",
        [
            IDC_STAND_LENGTH,
            IDC_STAND_LENGTH_SPIN,
            IDC_STAND_LENGTH_LABEL,
            IDC_STAND_LENGTH_UNIT,
            IDC_STAND_LEVEL,
        ],
    );
    let pause = row(
        "Pause",
        [
            IDC_PAUSE_ON,
            IDC_PAUSE_EVERY,
            IDC_PAUSE_EVERY_SPIN,
            IDC_PAUSE_EVERY_LABEL,
            IDC_PAUSE_EVERY_UNIT,
        ],
    ) + &length(
        "pause",
        [
            IDC_PAUSE_LENGTH,
            IDC_PAUSE_LENGTH_SPIN,
            IDC_PAUSE_LENGTH_LABEL,
            IDC_PAUSE_LENGTH_UNIT,
            IDC_PAUSE_LEVEL,
        ],
    );
    let water = row(
        "Water",
        [
            IDC_WATER_ON,
            IDC_WATER_EVERY,
            IDC_WATER_EVERY_SPIN,
            IDC_WATER_EVERY_LABEL,
            IDC_WATER_EVERY_UNIT,
        ],
    );
    format!(
        r#"
#include <windows.h>
#include <commctrl.h>

{IDD_SETTINGS} DIALOGEX 0, 0, 300, 200
STYLE DS_MODALFRAME | DS_CENTER | WS_POPUP | WS_CAPTION | WS_SYSMENU
EXSTYLE WS_EX_APPWINDOW
CAPTION "breakpoint settings"
BEGIN
    LTEXT "Work week", {IDC_WEEK_HEADING}, {PLACED}
    LTEXT "Work days", {IDC_DAYS_LABEL}, {PLACED}
{days}    LTEXT "Work hours", {IDC_HOURS_LABEL}, {PLACED}
    CONTROL "", {IDC_FROM}, "SysDateTimePick32", DTS_TIMEFORMAT | DTS_UPDOWN | WS_TABSTOP, {PLACED}
    CTEXT "to", {IDC_HOURS_TO}, {PLACED}
    CONTROL "", {IDC_TO}, "SysDateTimePick32", DTS_TIMEFORMAT | DTS_UPDOWN | WS_TABSTOP, {PLACED}

    LTEXT "Nudges", {IDC_NUDGES_HEADING}, {PLACED}
    LTEXT "Profile", {IDC_PROFILE_LABEL}, {PLACED}
    COMBOBOX {IDC_PROFILE}, {PLACED}, CBS_DROPDOWNLIST | WS_VSCROLL | WS_TABSTOP
{stand}{pause}{water}    LTEXT "Always gentle; rides along with the next nudge.", {IDC_WATER_NOTE}, {PLACED}

    AUTOCHECKBOX "Start with Windows", {IDC_START_WITH_WINDOWS}, {PLACED}, WS_GROUP | WS_TABSTOP
    CONTROL "Save", IDOK, "Button", BS_OWNERDRAW | WS_GROUP | WS_TABSTOP, {PLACED}
    PUSHBUTTON "Cancel", IDCANCEL, {PLACED}
END
"#
    )
}
