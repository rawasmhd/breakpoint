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
    // Icon 1 is the exe icon and the tray's normal state; icon 2 is the greyed (quiet) state.
    res.set_icon_with_id("assets/breakpoint.ico", "1");
    res.set_icon_with_id("assets/breakpoint-grey.ico", "2");
    res.set_manifest_file("assets/breakpoint.exe.manifest");
    // The SDK headers give the template its style names.
    res.add_toolkit_include(true);
    res.append_rc_content(&settings_dialog());
    res.compile().expect("failed to embed Windows resources");
}

/// The settings window (SPEC §9.2): one fixed-size page, in dialog units. Tab order is the
/// order below; each up-down control follows its edit box, which it takes as its buddy.
fn settings_dialog() -> String {
    use ids::*;
    let days = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"]
        .iter()
        .enumerate()
        .map(|(i, day)| {
            format!(
                "    AUTOCHECKBOX \"{day}\", {}, {}, 20, 30, 10\n",
                IDC_DAY + i as i32,
                62 + 32 * i
            )
        })
        .collect::<String>();
    let spin = |id: i32| {
        format!(
            "    CONTROL \"\", {id}, \"msctls_updown32\", UDS_AUTOBUDDY | UDS_SETBUDDYINT | \
             UDS_ALIGNRIGHT | UDS_ARROWKEYS | UDS_NOTHOUSANDS, 0, 0, 0, 0\n"
        )
    };
    let away_row = |y: i32, name: &str, length: &str, ids: [i32; 6]| {
        let [on, every, every_spin, len, len_spin, level] = ids;
        format!(
            "    AUTOCHECKBOX \"{name}\", {on}, 14, {y}, 54, 10\n\
             \x20   LTEXT \"every\", -1, 70, {ty}, 20, 8\n\
             \x20   EDITTEXT {every}, 92, {ey}, 32, 12, ES_NUMBER\n\
             {}\
             \x20   LTEXT \"min, {length} length\", -1, 128, {ty}, 64, 8\n\
             \x20   EDITTEXT {len}, 194, {ey}, 28, 12, ES_NUMBER\n\
             {}\
             \x20   LTEXT \"min\", -1, 226, {ty}, 14, 8\n\
             \x20   COMBOBOX {level}, 244, {cy}, 42, 40, CBS_DROPDOWNLIST | WS_VSCROLL | WS_TABSTOP\n",
            spin(every_spin),
            spin(len_spin),
            ty = y + 1,
            ey = y - 1,
            cy = y - 2,
        )
    };
    let stand = away_row(
        100,
        "Stand/Move",
        "stand",
        [
            IDC_STAND_ON,
            IDC_STAND_EVERY,
            IDC_STAND_EVERY_SPIN,
            IDC_STAND_LENGTH,
            IDC_STAND_LENGTH_SPIN,
            IDC_STAND_LEVEL,
        ],
    );
    let pause = away_row(
        118,
        "Pause",
        "pause",
        [
            IDC_PAUSE_ON,
            IDC_PAUSE_EVERY,
            IDC_PAUSE_EVERY_SPIN,
            IDC_PAUSE_LENGTH,
            IDC_PAUSE_LENGTH_SPIN,
            IDC_PAUSE_LEVEL,
        ],
    );
    let water_spin = spin(IDC_WATER_EVERY_SPIN);
    format!(
        r#"
#include <windows.h>
#include <commctrl.h>

{IDD_SETTINGS} DIALOGEX 0, 0, 300, 222
STYLE DS_SHELLFONT | DS_MODALFRAME | DS_CENTER | WS_POPUP | WS_CAPTION | WS_SYSMENU
EXSTYLE WS_EX_APPWINDOW
CAPTION "breakpoint settings"
FONT 9, "Segoe UI", 400, 0, 0x1
BEGIN
    GROUPBOX "Work week", -1, 7, 7, 286, 54
    LTEXT "Work days:", -1, 14, 21, 44, 8
{days}    LTEXT "Work hours from", -1, 14, 41, 56, 8
    CONTROL "", {IDC_FROM}, "SysDateTimePick32", DTS_TIMEFORMAT | DTS_UPDOWN | WS_TABSTOP, 72, 39, 44, 14
    LTEXT "to", -1, 122, 41, 10, 8
    CONTROL "", {IDC_TO}, "SysDateTimePick32", DTS_TIMEFORMAT | DTS_UPDOWN | WS_TABSTOP, 136, 39, 44, 14

    GROUPBOX "Nudges", -1, 7, 66, 286, 106
    LTEXT "Profile:", -1, 14, 81, 44, 8
    COMBOBOX {IDC_PROFILE}, 62, 79, 70, 60, CBS_DROPDOWNLIST | WS_VSCROLL | WS_TABSTOP
{stand}{pause}    AUTOCHECKBOX "Water", {IDC_WATER_ON}, 14, 136, 54, 10
    LTEXT "every", -1, 70, 137, 20, 8
    EDITTEXT {IDC_WATER_EVERY}, 92, 135, 32, 12, ES_NUMBER
{water_spin}    LTEXT "min", -1, 128, 137, 14, 8
    LTEXT "Always gentle; rides along with the next nudge.", -1, 70, 154, 216, 8

    AUTOCHECKBOX "Start with Windows", {IDC_START_WITH_WINDOWS}, 14, 180, 120, 10
    DEFPUSHBUTTON "Save", IDOK, 186, 201, 50, 14
    PUSHBUTTON "Cancel", IDCANCEL, 243, 201, 50, 14
END
"#
    )
}
