//! Embeds the exe icons, version info, application manifest and the wizard's dialog template.

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
    res.append_rc_content(&wizard_dialog());
    res.compile().expect("failed to embed Windows resources");
}

/// Every control is at 0, 0, 0, 0 here: `dialogs.rs` places each one in pixels from the design
/// tokens, for the monitor's DPI, and sets the fonts. So the template gives no font, and its
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
#include <windows.h>

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
