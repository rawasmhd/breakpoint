//! The first-run wizard and settings window DIALOGEX procedures.
//!
//! Both are modeless dialogs from templates that `build.rs` embeds, at most one of each at a
//! time, run by the app's own message loop through [`is_dialog_message`].
//!
//! The settings window (SPEC §9.2) lets nudges keep firing while it is open. It copies its
//! controls into a [`Form`] and back; the core decides the profile shown, what picking one fills
//! in and whether a Save is valid. A valid Save closes the window and posts [`WM_SETTINGS_SAVED`]
//! to the owner, which collects it with [`take_saved`] and applies it from its own window
//! procedure. Cancel, Esc or X posts [`WM_SETTINGS_CANCELLED`] instead.
//!
//! The first-run wizard (SPEC §9.1) shows the lines the core writes and reads back the
//! [`Picks`]. However it closes, it posts [`WM_WIZARD_DONE`], and the owner collects how with
//! [`take_wizard`].
//!
//! Both look as DESIGN.md §6 says, with the tokens of [`theme`]: the templates only make the
//! native controls, and this module places them in pixels for the monitor's DPI, gives them the
//! type styles, paints the `surface` ground and the `surface-sunken` footer band, and draws the
//! one primary button (Start, Save). Both stay light in every Windows theme, because native
//! controls have no supported dark mode.

use std::cell::{Cell, RefCell};
use std::time::{Duration, Instant};

use breakpoint::core::config::{Enforcement, Profile};
use breakpoint::core::settings::{AwayRow, Field, Form, Settings};
use breakpoint::core::time::{TimeOfDay, Weekday};
use breakpoint::core::wizard::{self, Picks};
use windows::Win32::Foundation::{
    COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, SYSTEMTIME, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreatePen, CreateSolidBrush, DT_CENTER,
    DT_SINGLELINE, DT_VCENTER, DeleteDC, DeleteObject, DrawTextW, Ellipse, FillRect,
    GetMonitorInfoW, GetStockObject, HBRUSH, HDC, HFONT, InvalidateRect, MONITOR_DEFAULTTONEAREST,
    MONITORINFO, MapWindowPoints, MonitorFromWindow, NULL_BRUSH, PS_INSIDEFRAME, RDW_ALLCHILDREN,
    RDW_ERASE, RDW_INVALIDATE, RedrawWindow, RoundRect, SRCCOPY, SelectObject, SetBkColor,
    SetBkMode, SetTextColor, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::UI::Controls::{
    BST_CHECKED, BST_UNCHECKED, CheckDlgButton, CheckRadioButton, DRAWITEMSTRUCT,
    DTM_GETSYSTEMTIME, DTM_SETFORMATW, DTM_SETSYSTEMTIME, EM_SETLIMITTEXT, GDT_VALID,
    ICC_DATE_CLASSES, ICC_STANDARD_CLASSES, ICC_UPDOWN_CLASS, INITCOMMONCONTROLSEX,
    InitCommonControlsEx, IsDlgButtonChecked, ODS_FOCUS, ODS_NOFOCUSRECT, ODS_SELECTED, ODT_BUTTON,
    UDM_SETBUDDY, UDM_SETRANGE32, WM_MOUSELEAVE,
};
use windows::Win32::UI::HiDpi::{
    AdjustWindowRectExForDpi, DDC_DISABLE_ALL, GetDpiForWindow, GetSystemMetricsForDpi,
    SetDialogDpiChangeBehavior,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent};
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    CB_ADDSTRING, CB_DELETESTRING, CB_GETCOUNT, CB_GETCURSEL, CB_SETCURSEL, CBN_SELCHANGE,
    CreateDialogParamW, DM_REPOSITION, DM_SETDEFID, DWLP_MSGRESULT, DestroyWindow, EN_CHANGE,
    GWL_EXSTYLE, GWL_STYLE, GetClientRect, GetDlgCtrlID, GetDlgItem, GetDlgItemTextW,
    GetSystemMetrics, GetWindowLongW, GetWindowRect, GetWindowTextW, ICON_BIG, ICON_SMALL,
    IDCANCEL, IDOK, IMAGE_ICON, IsDialogMessageW, IsIconic, KillTimer, LR_DEFAULTCOLOR, LoadImageW,
    MB_ICONWARNING, MB_OK, MSG, MessageBoxW, PostMessageW, SM_CXICON, SM_CXSMICON, SM_CXVSCROLL,
    SM_CYICON, SM_CYSMICON, STN_CLICKED, SW_RESTORE, SW_SHOW, SWP_NOACTIVATE, SWP_NOMOVE,
    SWP_NOSIZE, SWP_NOZORDER, SendDlgItemMessageW, SendMessageW, SetDlgItemTextW,
    SetForegroundWindow, SetTimer, SetWindowLongPtrW, SetWindowPos, ShowWindow, WINDOW_EX_STYLE,
    WINDOW_LONG_PTR_INDEX, WINDOW_STYLE, WM_APP, WM_CLOSE, WM_COMMAND, WM_CTLCOLORBTN,
    WM_CTLCOLORDLG, WM_CTLCOLORSTATIC, WM_DESTROY, WM_DPICHANGED, WM_DRAWITEM, WM_ERASEBKGND,
    WM_INITDIALOG, WM_MOUSEMOVE, WM_NCDESTROY, WM_NEXTDLGCTL, WM_PRINTCLIENT, WM_SETFONT,
    WM_SETICON, WM_TIMER,
};
use windows::core::{HSTRING, PCWSTR, w};

use super::theme::{self, FontCache, LIGHT, Style, px};

mod ids {
    include!("settings_ids.rs");
}
use ids::*;

/// Asks the main window to open Settings, from a second launch (SPEC §11).
pub const WM_OPEN_SETTINGS: u32 = WM_APP + 3;
/// Posted to the owner when a valid Save closed the settings window; see [`take_saved`].
pub const WM_SETTINGS_SAVED: u32 = WM_APP + 4;
/// Posted to the owner when the settings window closed without a Save: Cancel, Esc or X.
pub const WM_SETTINGS_CANCELLED: u32 = WM_APP + 5;
/// Posted to the owner when the wizard closed; see [`take_wizard`].
pub const WM_WIZARD_DONE: u32 = WM_APP + 6;

/// How the wizard closed.
pub enum WizardDone {
    /// Start, X or Esc: accept the picks.
    Start(Picks),
    /// Change…: open the settings window with the picks, its top-left at this screen point,
    /// where the wizard's was.
    Change(Picks, POINT),
}

/// What a valid Save hands the app.
pub struct Saved {
    pub settings: Settings,
    pub start_with_windows: bool,
}

/// What the dialog starts with, passed to `WM_INITDIALOG`.
struct Init<'a> {
    form: &'a Form,
    start_with_windows: bool,
}

thread_local! {
    /// The settings window while it is open.
    static OPEN: Cell<Option<HWND>> = const { Cell::new(None) };
    /// Where a valid Save is posted.
    static OWNER: Cell<Option<HWND>> = const { Cell::new(None) };
    /// The dialog itself is writing its controls: their change notifications are not edits.
    static FILLING: Cell<bool> = const { Cell::new(false) };
    static SAVED: RefCell<Option<Saved>> = const { RefCell::new(None) };
    /// The wizard while it is open.
    static WIZARD: Cell<Option<HWND>> = const { Cell::new(None) };
    static WIZARD_DONE: RefCell<Option<WizardDone>> = const { RefCell::new(None) };
}

/// The profile drop-down's items in order; Custom is added only while nothing matches.
const PROFILES: [(Profile, PCWSTR); 3] = [
    (Profile::Gentle, w!("Gentle")),
    (Profile::Balanced, w!("Balanced")),
    (Profile::Strict, w!("Strict")),
];
const CUSTOM: usize = PROFILES.len();

/// The exe icon's resource id (see `build.rs`).
const APP_ICON: u16 = 1;

const STAND: Row = Row {
    on: IDC_STAND_ON,
    every: IDC_STAND_EVERY,
    every_spin: IDC_STAND_EVERY_SPIN,
    length: IDC_STAND_LENGTH,
    length_spin: IDC_STAND_LENGTH_SPIN,
    level: IDC_STAND_LEVEL,
};
const PAUSE: Row = Row {
    on: IDC_PAUSE_ON,
    every: IDC_PAUSE_EVERY,
    every_spin: IDC_PAUSE_EVERY_SPIN,
    length: IDC_PAUSE_LENGTH,
    length_spin: IDC_PAUSE_LENGTH_SPIN,
    level: IDC_PAUSE_LEVEL,
};

/// The controls of the Stand/Move or Pause row.
struct Row {
    on: i32,
    every: i32,
    every_spin: i32,
    length: i32,
    length_spin: i32,
    level: i32,
}

/// Opens the settings window filled with `form`, or brings it to the front if it is open
/// (SPEC §9.2). `owner` receives [`WM_SETTINGS_SAVED`] or [`WM_SETTINGS_CANCELLED`]. With `at`,
/// the window's top-left goes there, kept on screen; otherwise it is centred.
pub fn open_settings(owner: HWND, form: &Form, start_with_windows: bool, at: Option<POINT>) {
    if let Some(dialog) = OPEN.get() {
        bring_to_front(dialog);
        return;
    }
    OWNER.set(Some(owner));
    let init = Init {
        form,
        start_with_windows,
    };
    // SAFETY: the template is embedded by build.rs; `init` outlives the call, during which
    // WM_INITDIALOG reads it. No parent, so the window has its own taskbar button.
    unsafe {
        let _ = InitCommonControlsEx(&INITCOMMONCONTROLSEX {
            dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_STANDARD_CLASSES | ICC_DATE_CLASSES | ICC_UPDOWN_CLASS,
        });
        let Ok(instance) = GetModuleHandleW(None) else {
            return;
        };
        let Ok(dialog) = CreateDialogParamW(
            Some(instance.into()),
            resource(IDD_SETTINGS),
            None,
            Some(settings_proc),
            LPARAM(&init as *const Init as isize),
        ) else {
            return;
        };
        OPEN.set(Some(dialog));
        if let Some(at) = at {
            let _ = SetWindowPos(
                dialog,
                None,
                at.x,
                at.y,
                0,
                0,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
            SendMessageW(dialog, DM_REPOSITION, None, None);
        }
        let _ = ShowWindow(dialog, SW_SHOW);
        let _ = SetForegroundWindow(dialog);
    }
}

/// Opens the first-run wizard with `picks` preselected (SPEC §9.1). `owner` receives
/// [`WM_WIZARD_DONE`] when it closes.
pub fn open_wizard(owner: HWND, picks: Picks) {
    if let Some(dialog) = WIZARD.get() {
        bring_to_front(dialog);
        return;
    }
    OWNER.set(Some(owner));
    // SAFETY: the template is embedded by build.rs; `picks` outlives the call, during which
    // WM_INITDIALOG reads it. No parent, so the window has its own taskbar button.
    unsafe {
        let _ = InitCommonControlsEx(&INITCOMMONCONTROLSEX {
            dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_STANDARD_CLASSES,
        });
        let Ok(instance) = GetModuleHandleW(None) else {
            return;
        };
        let Ok(dialog) = CreateDialogParamW(
            Some(instance.into()),
            resource(IDD_WIZARD),
            None,
            Some(wizard_proc),
            LPARAM(&picks as *const Picks as isize),
        ) else {
            return;
        };
        WIZARD.set(Some(dialog));
        let _ = ShowWindow(dialog, SW_SHOW);
        let _ = SetForegroundWindow(dialog);
    }
}

/// How the wizard closed, once, after [`WM_WIZARD_DONE`].
pub fn take_wizard() -> Option<WizardDone> {
    WIZARD_DONE.take()
}

/// Brings the wizard, or else the settings window, to the front: a second launch during first
/// run (SPEC §11). False if neither is open.
pub fn to_front() -> bool {
    match WIZARD.get().or(OPEN.get()) {
        Some(dialog) => {
            bring_to_front(dialog);
            true
        }
        None => false,
    }
}

/// Lets an open dialog handle `msg` (Tab, Enter, Esc, arrows); true if it did, and then the
/// message must not be translated or dispatched again.
pub fn is_dialog_message(msg: &MSG) -> bool {
    // SAFETY: `dialog` is a live dialog; `msg` is a valid MSG.
    [WIZARD.get(), OPEN.get()]
        .into_iter()
        .flatten()
        .any(|dialog| unsafe { IsDialogMessageW(dialog, msg) }.as_bool())
}

/// The Save that [`WM_SETTINGS_SAVED`] announced, once.
pub fn take_saved() -> Option<Saved> {
    SAVED.take()
}

/// One message box with no owner, for what went wrong after the window closed.
pub fn error(text: &str) {
    // SAFETY: both strings live through the call.
    unsafe {
        MessageBoxW(
            None,
            &HSTRING::from(text),
            w!("breakpoint"),
            MB_OK | MB_ICONWARNING,
        );
    }
}

fn bring_to_front(dialog: HWND) {
    // SAFETY: `dialog` is a live dialog.
    unsafe {
        if IsIconic(dialog).as_bool() {
            let _ = ShowWindow(dialog, SW_RESTORE);
        }
        let _ = SetForegroundWindow(dialog);
    }
}

unsafe extern "system" fn settings_proc(
    dialog: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> isize {
    if let Some(result) = look(dialog, Kind::Settings, msg, wparam, lparam) {
        return result;
    }
    match msg {
        WM_INITDIALOG => {
            // SAFETY: `open_settings` passes a pointer to an `Init` that outlives this call.
            let init = unsafe { &*(lparam.0 as *const Init) };
            init_controls(dialog);
            write_form(dialog, init.form);
            check(dialog, IDC_START_WITH_WINDOWS, init.start_with_windows);
            set_up(dialog, Kind::Settings);
            // Focus goes to the first control: Sunday's checkbox.
            1
        }
        WM_COMMAND => {
            let id = (wparam.0 & 0xffff) as i32;
            let code = ((wparam.0 >> 16) & 0xffff) as u32;
            // The up-down controls write their edit boxes while the dialog is being created,
            // before WM_INITDIALOG: no change counts as an edit until the dialog is open.
            let edited = OPEN.get() == Some(dialog) && !FILLING.get();
            match (id, code) {
                (id, _) if id == IDOK.0 => save(dialog),
                (id, _) if id == IDCANCEL.0 => cancel(dialog),
                (IDC_PROFILE, CBN_SELCHANGE) if edited => pick_profile(dialog),
                (_, EN_CHANGE | CBN_SELCHANGE) if edited => show_profile(dialog),
                _ => return 0,
            }
            1
        }
        // X discards without asking, as Cancel does.
        WM_CLOSE => {
            cancel(dialog);
            1
        }
        WM_DESTROY => {
            OPEN.set(None);
            0
        }
        _ => 0,
    }
}

unsafe extern "system" fn wizard_proc(
    dialog: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> isize {
    if let Some(result) = look(dialog, Kind::Wizard, msg, wparam, lparam) {
        return result;
    }
    match msg {
        WM_INITDIALOG => {
            // SAFETY: `open_wizard` passes a pointer to `Picks` that outlive this call.
            let picks = unsafe { *(lparam.0 as *const Picks) };
            set_icons(dialog);
            for (i, profile) in (0..).zip(Profile::ALL) {
                set_text(dialog, IDC_WIZARD_PROFILE + i, profile_name(profile));
                set_text(dialog, IDC_WIZARD_NUMBERS + i, &numbers_line(profile));
            }
            let picked = profile_radio(picks.profile);
            // SAFETY: the ids name the three radio buttons of `dialog`.
            let _ = unsafe {
                CheckRadioButton(dialog, IDC_WIZARD_PROFILE, IDC_WIZARD_PROFILE + 2, picked)
            };
            check(
                dialog,
                IDC_WIZARD_START_WITH_WINDOWS,
                picks.start_with_windows,
            );
            set_text(
                dialog,
                IDC_WIZARD_WEEK,
                &wizard::work_week_line(&picks.config()),
            );
            set_text(dialog, IDC_WIZARD_CHANGE, "Change\u{2026}");
            set_up(dialog, Kind::Wizard);
            // Focus on the picked profile, where the arrow keys change it. Set here rather than
            // left to the dialog, which would focus the first radio button and so pick it.
            focus(dialog, picked);
            0
        }
        WM_COMMAND => {
            let id = (wparam.0 & 0xffff) as i32;
            let code = ((wparam.0 >> 16) & 0xffff) as u32;
            match (id, code) {
                // Start, and Esc, which is closing: both accept what the page shows.
                (id, _) if id == IDOK.0 || id == IDCANCEL.0 => wizard_done(dialog, false),
                (IDC_WIZARD_CHANGE, _) => wizard_done(dialog, true),
                // A click on a numbers line picks its profile, as a click on the name does.
                (id, STN_CLICKED) if (IDC_WIZARD_NUMBERS..IDC_WIZARD_NUMBERS + 3).contains(&id) => {
                    let radio = id - IDC_WIZARD_NUMBERS + IDC_WIZARD_PROFILE;
                    // SAFETY: the ids name the three radio buttons of `dialog`.
                    let _ = unsafe {
                        CheckRadioButton(dialog, IDC_WIZARD_PROFILE, IDC_WIZARD_PROFILE + 2, radio)
                    };
                    focus(dialog, radio);
                }
                _ => return 0,
            }
            1
        }
        // X is the same as Start.
        WM_CLOSE => {
            wizard_done(dialog, false);
            1
        }
        WM_DESTROY => {
            WIZARD.set(None);
            0
        }
        _ => 0,
    }
}

/// The radio button of `profile`.
fn profile_radio(profile: Profile) -> i32 {
    let index = Profile::ALL.iter().position(|&p| p == profile).unwrap_or(0);
    IDC_WIZARD_PROFILE + index as i32
}

/// The wizard closes, by Start, X, Esc or Change…: hand the picks over and go.
fn wizard_done(dialog: HWND, change: bool) {
    let picks = Picks {
        profile: Profile::ALL
            .into_iter()
            .find(|&p| checked(dialog, profile_radio(p)))
            .unwrap_or(Picks::default().profile),
        start_with_windows: checked(dialog, IDC_WIZARD_START_WITH_WINDOWS),
    };
    let done = if change {
        let mut rect = RECT::default();
        // SAFETY: `dialog` is the live wizard; `rect` is a valid out pointer.
        let _ = unsafe { GetWindowRect(dialog, &mut rect) };
        let at = POINT {
            x: rect.left,
            y: rect.top,
        };
        WizardDone::Change(picks, at)
    } else {
        WizardDone::Start(picks)
    };
    WIZARD_DONE.set(Some(done));
    post(WM_WIZARD_DONE);
    close(dialog);
}

/// Posts `msg` to the owner.
fn post(msg: u32) {
    if let Some(owner) = OWNER.get() {
        // SAFETY: `owner` is the app's main window, alive as long as the app.
        let _ = unsafe { PostMessageW(Some(owner), msg, WPARAM(0), LPARAM(0)) };
    }
}

/// The app icon in the caption and on the taskbar.
fn set_icons(dialog: HWND) {
    // SAFETY: `dialog` is the window being initialised.
    unsafe {
        let instance = GetModuleHandleW(None).ok().map(Into::into);
        for (size, which) in [
            ((SM_CXICON, SM_CYICON), ICON_BIG),
            ((SM_CXSMICON, SM_CYSMICON), ICON_SMALL),
        ] {
            if let Ok(icon) = LoadImageW(
                instance,
                resource(APP_ICON),
                IMAGE_ICON,
                GetSystemMetrics(size.0),
                GetSystemMetrics(size.1),
                LR_DEFAULTCOLOR,
            ) {
                SendMessageW(
                    dialog,
                    WM_SETICON,
                    Some(WPARAM(which as usize)),
                    Some(LPARAM(icon.0 as isize)),
                );
            }
        }
    }
}

fn init_controls(dialog: HWND) {
    set_icons(dialog);
    // Every string sent is static.
    for id in [IDC_FROM, IDC_TO] {
        send(dialog, id, DTM_SETFORMATW, 0, w!("HH:mm").as_ptr() as isize);
    }
    for (_, name) in PROFILES {
        send(dialog, IDC_PROFILE, CB_ADDSTRING, 0, name.as_ptr() as isize);
    }
    for level in [STAND.level, PAUSE.level] {
        for name in [w!("Soft"), w!("Firm")] {
            send(dialog, level, CB_ADDSTRING, 0, name.as_ptr() as isize);
        }
    }
    use breakpoint::core::config::{
        PAUSE_EVERY, PAUSE_LENGTH, STAND_EVERY, STAND_LENGTH, WATER_EVERY,
    };
    for (edit, spin, range) in [
        (STAND.every, STAND.every_spin, STAND_EVERY),
        (STAND.length, STAND.length_spin, STAND_LENGTH),
        (PAUSE.every, PAUSE.every_spin, PAUSE_EVERY),
        (PAUSE.length, PAUSE.length_spin, PAUSE_LENGTH),
        (IDC_WATER_EVERY, IDC_WATER_EVERY_SPIN, WATER_EVERY),
    ] {
        send(dialog, edit, EM_SETLIMITTEXT, 3, 0);
        send(
            dialog,
            spin,
            UDM_SETRANGE32,
            *range.start() as usize,
            *range.end() as isize,
        );
    }
}

/// Puts `form` into the controls, then shows its profile.
fn write_form(dialog: HWND, form: &Form) {
    FILLING.set(true);
    for (i, day) in Weekday::ALL.into_iter().enumerate() {
        check(dialog, IDC_DAY + i as i32, form.work_days.contains(day));
    }
    set_time(dialog, IDC_FROM, form.start);
    set_time(dialog, IDC_TO, form.end);
    write_row(dialog, &STAND, &form.stand);
    write_row(dialog, &PAUSE, &form.pause);
    check(dialog, IDC_WATER_ON, form.water.on);
    set_text(dialog, IDC_WATER_EVERY, &form.water.every);
    FILLING.set(false);
    show_profile(dialog);
}

fn write_row(dialog: HWND, row: &Row, values: &AwayRow) {
    check(dialog, row.on, values.on);
    set_text(dialog, row.every, &values.every);
    set_text(dialog, row.length, &values.length);
    let level = match values.enforcement {
        Enforcement::Soft => 0,
        Enforcement::Firm => 1,
    };
    send(dialog, row.level, CB_SETCURSEL, level, 0);
}

/// The controls as a form.
fn read_form(dialog: HWND) -> Form {
    let mut form = Form::new(&Default::default());
    for (i, day) in Weekday::ALL.into_iter().enumerate() {
        form.work_days.set(day, checked(dialog, IDC_DAY + i as i32));
    }
    form.start = time(dialog, IDC_FROM).unwrap_or(form.start);
    form.end = time(dialog, IDC_TO).unwrap_or(form.end);
    form.stand = read_row(dialog, &STAND);
    form.pause = read_row(dialog, &PAUSE);
    form.water.on = checked(dialog, IDC_WATER_ON);
    form.water.every = text(dialog, IDC_WATER_EVERY);
    form
}

fn read_row(dialog: HWND, row: &Row) -> AwayRow {
    AwayRow {
        on: checked(dialog, row.on),
        every: text(dialog, row.every),
        length: text(dialog, row.length),
        enforcement: if send(dialog, row.level, CB_GETCURSEL, 0, 0) == 1 {
            Enforcement::Firm
        } else {
            Enforcement::Soft
        },
    }
}

/// Shows in the drop-down the profile the form matches, or Custom, which is in the list only
/// while it is the one shown.
fn show_profile(dialog: HWND) {
    let profile = read_form(dialog).profile();
    let index = PROFILES
        .iter()
        .position(|&(p, _)| Some(p) == profile)
        .unwrap_or(CUSTOM);
    let has_custom = send(dialog, IDC_PROFILE, CB_GETCOUNT, 0, 0) > CUSTOM as isize;
    if index == CUSTOM && !has_custom {
        send(
            dialog,
            IDC_PROFILE,
            CB_ADDSTRING,
            0,
            w!("Custom").as_ptr() as isize,
        );
    } else if index != CUSTOM && has_custom {
        send(dialog, IDC_PROFILE, CB_DELETESTRING, CUSTOM, 0);
    }
    send(dialog, IDC_PROFILE, CB_SETCURSEL, index, 0);
}

/// A profile picked in the drop-down fills the form only; Custom picked changes nothing.
fn pick_profile(dialog: HWND) {
    let index = send(dialog, IDC_PROFILE, CB_GETCURSEL, 0, 0);
    if let Some(&(profile, _)) = usize::try_from(index).ok().and_then(|i| PROFILES.get(i)) {
        let mut form = read_form(dialog);
        form.apply(profile);
        write_form(dialog, &form);
    } else {
        show_profile(dialog);
    }
}

/// Save: refuse with one message box and the focus on the field, or hand the settings over
/// and close.
fn save(dialog: HWND) {
    match read_form(dialog).validate() {
        Err(invalid) => {
            // SAFETY: `dialog` is live; the strings live through the calls.
            unsafe {
                MessageBoxW(
                    Some(dialog),
                    &HSTRING::from(invalid.message),
                    w!("breakpoint settings"),
                    MB_OK | MB_ICONWARNING,
                );
            }
            focus(dialog, control_of(invalid.field));
        }
        Ok(settings) => {
            SAVED.set(Some(Saved {
                settings,
                start_with_windows: checked(dialog, IDC_START_WITH_WINDOWS),
            }));
            post(WM_SETTINGS_SAVED);
            close(dialog);
        }
    }
}

/// The control a refused Save focuses.
fn control_of(field: Field) -> i32 {
    match field {
        Field::WorkDays => IDC_DAY,
        Field::WorkHoursEnd => IDC_TO,
        Field::StandEvery => STAND.every,
        Field::StandLength => STAND.length,
        Field::PauseEvery => PAUSE.every,
        Field::PauseLength => PAUSE.length,
        Field::WaterEvery => IDC_WATER_EVERY,
    }
}

/// Cancel, Esc or X: discard the edits, and tell the owner, for whom it may end first run.
fn cancel(dialog: HWND) {
    post(WM_SETTINGS_CANCELLED);
    close(dialog);
}

fn close(dialog: HWND) {
    // SAFETY: `dialog` is a live dialog; WM_DESTROY forgets it.
    let _ = unsafe { DestroyWindow(dialog) };
}

fn send(dialog: HWND, id: i32, msg: u32, wparam: usize, lparam: isize) -> isize {
    // SAFETY: `id` names a control of `dialog`; any pointer in `lparam` outlives the call.
    unsafe { SendDlgItemMessageW(dialog, id, msg, WPARAM(wparam), LPARAM(lparam)) }.0
}

fn check(dialog: HWND, id: i32, on: bool) {
    let state = if on { BST_CHECKED } else { BST_UNCHECKED };
    // SAFETY: `id` names a checkbox of `dialog`.
    let _ = unsafe { CheckDlgButton(dialog, id, state) };
}

fn checked(dialog: HWND, id: i32) -> bool {
    // SAFETY: `id` names a checkbox of `dialog`.
    unsafe { IsDlgButtonChecked(dialog, id) == BST_CHECKED.0 }
}

fn text(dialog: HWND, id: i32) -> String {
    let mut buf = [0u16; 16];
    // SAFETY: the buffer is valid for its length.
    let len = unsafe { GetDlgItemTextW(dialog, id, &mut buf) } as usize;
    String::from_utf16_lossy(&buf[..len.min(buf.len())])
}

fn set_text(dialog: HWND, id: i32, text: &str) {
    // SAFETY: the string lives through the call.
    let _ = unsafe { SetDlgItemTextW(dialog, id, &HSTRING::from(text)) };
}

/// A date-time picker's time of day.
fn time(dialog: HWND, id: i32) -> Option<TimeOfDay> {
    let mut t = SYSTEMTIME::default();
    let got = send(dialog, id, DTM_GETSYSTEMTIME, 0, &mut t as *mut _ as isize);
    if got != GDT_VALID.0 as isize {
        return None;
    }
    TimeOfDay::new(t.wHour as u8, t.wMinute as u8)
}

fn set_time(dialog: HWND, id: i32, time: TimeOfDay) {
    // The picker needs a whole date; today's will do, as only the time is shown.
    // SAFETY: plain call returning a SYSTEMTIME.
    let mut t = unsafe { GetLocalTime() };
    t.wHour = time.hour().into();
    t.wMinute = time.minute().into();
    (t.wSecond, t.wMilliseconds) = (0, 0);
    send(
        dialog,
        id,
        DTM_SETSYSTEMTIME,
        GDT_VALID.0 as usize,
        &t as *const _ as isize,
    );
}

/// MAKEINTRESOURCE: a resource id in place of a name.
fn resource(id: u16) -> PCWSTR {
    PCWSTR(usize::from(id) as *const u16)
}

/// Moves the focus the dialog's way to control `id`, which also selects an edit box's text.
fn focus(dialog: HWND, id: i32) {
    // SAFETY: `dialog` is a live dialog.
    unsafe {
        if let Ok(control) = GetDlgItem(Some(dialog), id) {
            SendMessageW(
                dialog,
                WM_NEXTDLGCTL,
                Some(WPARAM(control.0 as usize)),
                Some(LPARAM(1)),
            );
        }
    }
}

/// A profile's name, the text of its radio button.
fn profile_name(profile: Profile) -> &'static str {
    match profile {
        Profile::Gentle => "Gentle",
        Profile::Balanced => "Balanced",
        Profile::Strict => "Strict",
    }
}

/// The core's line of a profile's numbers without the "Name: " in front, which the radio
/// button above it already shows, and so with a capital first letter.
fn numbers_line(profile: Profile) -> String {
    let line = wizard::profile_line(profile);
    let numbers = match line.split_once(": ") {
        Some((name, numbers)) if name == profile_name(profile) => numbers,
        _ => &line,
    };
    let mut chars = numbers.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

// ---- The look (DESIGN.md §6) ----
//
// Sizes are at 96 DPI, as in DESIGN.md, and scale with `px`. The tokens come from `theme`; the
// sizes named here are this layout's own: the label column, the field widths, the band.

/// Which dialog. Each keeps its own fonts, for the DPI of the monitor it is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Wizard,
    Settings,
}

/// Dialog padding.
const PAD: i32 = theme::SPACE_6;
/// Between sections.
const SECTION: i32 = theme::SPACE_6;
/// Between rows inside a section, and from a heading to its first row.
const ROW_GAP: i32 = theme::SPACE_2;
/// A row of inputs: edit boxes, drop-downs, time pickers and the checkboxes beside them.
const ROW: i32 = 24;
const LINE: i32 = Style::Body.line_height();
const CAPTION: i32 = Style::Caption.line_height();
/// Top of `body` text centred in a [`ROW`].
const TEXT_IN_ROW: i32 = (ROW - LINE) / 2;
/// The label column of the settings window: wide enough for the Stand/Move checkbox.
const LABEL: i32 = 104;
/// Where the controls right of the labels start.
const FIELD: i32 = PAD + LABEL + theme::SPACE_3;
/// Client width of both dialogs, so Change… opens the settings window where the wizard was.
const WIDTH: i32 = 596;
/// The text of a checkbox or radio button starts this far right of the control: its glyph and
/// the gap after it. Notes under a radio button or checkbox line up with its text.
const GLYPH: i32 = 20;
/// Width of each footer button and of Change…: at least `PRIMARY_MIN_WIDTH`.
const BUTTON: i32 = 88;
const _: () = assert!(BUTTON >= theme::PRIMARY_MIN_WIDTH);
/// Footer band height: a button and `space-4` above and below it.
const BAND: i32 = theme::SIZE_BUTTON + 2 * theme::SPACE_4;
/// The time pickers.
const TIME: i32 = 72;
/// A work-day checkbox, and the step from one to the next.
const DAY: i32 = 52;
const DAY_STEP: i32 = 56;
/// How far a combo box's list drops below it.
const DROP: i32 = 160;

/// One control's place at 96 DPI and its type style.
#[derive(Clone, Copy, Debug)]
struct Place {
    id: i32,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    style: Style,
}

/// A dialog at 96 DPI: client size, where the footer band starts, every placed control, and
/// the top-left of the `dot` (`size-mark` wide) where the dialog has one.
struct Layout {
    width: i32,
    height: i32,
    band: i32,
    places: Vec<Place>,
    dot: Option<(i32, i32)>,
}

/// The combo boxes, which are placed by their list's size and then centred in their row.
const COMBOS: [i32; 3] = [IDC_PROFILE, STAND.level, PAUSE.level];

/// Each up-down control and its edit box, which it sits in at the right edge.
const SPINS: [(i32, i32); 5] = [
    (STAND.every_spin, STAND.every),
    (STAND.length_spin, STAND.length),
    (PAUSE.every_spin, PAUSE.every),
    (PAUSE.length_spin, PAUSE.length),
    (IDC_WATER_EVERY_SPIN, IDC_WATER_EVERY),
];

impl Kind {
    fn layout(self) -> Layout {
        match self {
            Kind::Wizard => wizard_layout(),
            Kind::Settings => settings_layout(),
        }
    }
}

/// Collects places, top to bottom.
struct Builder(Vec<Place>);

impl Builder {
    fn put(&mut self, id: i32, (x, y): (i32, i32), (w, h): (i32, i32), style: Style) {
        self.0.push(Place {
            id,
            x,
            y,
            w,
            h,
            style,
        });
    }

    /// A section heading across the dialog; returns the top of the row under it.
    fn heading(&mut self, id: i32, y: i32) -> i32 {
        self.put(id, (PAD, y), (WIDTH - 2 * PAD, LINE), Style::BodyStrong);
        y + LINE + ROW_GAP
    }

    /// A label in the label column of a row at `y`.
    fn label(&mut self, id: i32, y: i32) {
        self.put(id, (PAD, y + TEXT_IN_ROW), (LABEL, LINE), Style::Body);
    }

    /// `body` text in a row at `y`.
    fn text(&mut self, id: i32, x: i32, y: i32, w: i32) {
        self.put(id, (x, y + TEXT_IN_ROW), (w, LINE), Style::Body);
    }

    /// The footer: buttons right-aligned, `space-2` apart, in the order given. Returns the
    /// dialog's height.
    fn footer(&mut self, band: i32, ids: &[i32]) -> i32 {
        let mut x = WIDTH - PAD - BUTTON;
        for &id in ids.iter().rev() {
            self.put(
                id,
                (x, band + theme::SPACE_4),
                (BUTTON, theme::SIZE_BUTTON),
                Style::BodyStrong,
            );
            x -= BUTTON + theme::SPACE_2;
        }
        band + BAND
    }
}

/// The first-run wizard: the heading, Profile, Work week, then Start with Windows.
fn wizard_layout() -> Layout {
    let mut b = Builder(Vec::new());
    let right = WIDTH - PAD;
    let title = Style::TitleLg.line_height();
    // The dot comes first, `space-2` before the heading, centred on its line box.
    let dot = (PAD, PAD + (title - theme::SIZE_MARK) / 2);
    let title_x = PAD + theme::SIZE_MARK + theme::SPACE_2;
    // GDI sets the 28px heading 37px tall, so its control gets `space-1` more than the line
    // box; the spacing below still counts from the line box.
    let title_box = (right - title_x, title + theme::SPACE_1);
    b.put(IDC_WIZARD_TITLE, (title_x, PAD), title_box, Style::TitleLg);
    let mut y = b.heading(IDC_WIZARD_PROFILE_HEADING, PAD + title + theme::SPACE_8);
    for i in 0..3 {
        if i > 0 {
            y += ROW_GAP;
        }
        let name = (right - PAD, LINE);
        b.put(IDC_WIZARD_PROFILE + i, (PAD, y), name, Style::BodyStrong);
        y += LINE;
        let numbers = (right - PAD - GLYPH, CAPTION);
        b.put(
            IDC_WIZARD_NUMBERS + i,
            (PAD + GLYPH, y),
            numbers,
            Style::Caption,
        );
        y += CAPTION;
    }
    y = b.heading(IDC_WIZARD_WEEK_HEADING, y + SECTION);
    let button = theme::SIZE_BUTTON;
    let line = (right - BUTTON - theme::SPACE_3 - PAD, LINE);
    b.put(
        IDC_WIZARD_WEEK,
        (PAD, y + (button - LINE) / 2),
        line,
        Style::Body,
    );
    let change = (BUTTON, button);
    b.put(
        IDC_WIZARD_CHANGE,
        (right - BUTTON, y),
        change,
        Style::BodyStrong,
    );
    y += button + SECTION;
    let check = (right - PAD, LINE);
    b.put(IDC_WIZARD_START_WITH_WINDOWS, (PAD, y), check, Style::Body);
    y += LINE;
    let note = (right - PAD - GLYPH, CAPTION);
    b.put(IDC_WIZARD_TRAY_NOTE, (PAD + GLYPH, y), note, Style::Caption);
    let band = y + CAPTION + PAD;
    let height = b.footer(band, &[IDOK.0]);
    Layout {
        width: WIDTH,
        height,
        band,
        places: b.0,
        dot: Some(dot),
    }
}

/// The settings window: Work week, Nudges, then Start with Windows. Labels on one column,
/// the units after the edit boxes, the Water note under its row.
fn settings_layout() -> Layout {
    let mut b = Builder(Vec::new());
    let right = WIDTH - PAD;

    let mut y = b.heading(IDC_WEEK_HEADING, PAD);
    b.label(IDC_DAYS_LABEL, y);
    for i in 0..7 {
        b.put(
            IDC_DAY + i,
            (FIELD + DAY_STEP * i, y),
            (DAY, ROW),
            Style::Body,
        );
    }
    y += ROW + ROW_GAP;
    b.label(IDC_HOURS_LABEL, y);
    b.put(IDC_FROM, (FIELD, y), (TIME, ROW), Style::Body);
    let to = FIELD + TIME + theme::SPACE_2;
    b.text(IDC_HOURS_TO, to, y, 20);
    b.put(
        IDC_TO,
        (to + 20 + theme::SPACE_2, y),
        (TIME, ROW),
        Style::Body,
    );
    y += ROW + SECTION;

    y = b.heading(IDC_NUDGES_HEADING, y);
    b.label(IDC_PROFILE_LABEL, y);
    b.put(IDC_PROFILE, (FIELD, y), (128, ROW), Style::Body);
    y += ROW + ROW_GAP;
    let nudges = [
        (
            STAND.on,
            [IDC_STAND_EVERY_LABEL, STAND.every, IDC_STAND_EVERY_UNIT],
        ),
        (
            PAUSE.on,
            [IDC_PAUSE_EVERY_LABEL, PAUSE.every, IDC_PAUSE_EVERY_UNIT],
        ),
        (
            IDC_WATER_ON,
            [IDC_WATER_EVERY_LABEL, IDC_WATER_EVERY, IDC_WATER_EVERY_UNIT],
        ),
    ];
    let lengths = [
        [
            IDC_STAND_LENGTH_LABEL,
            STAND.length,
            IDC_STAND_LENGTH_UNIT,
            STAND.level,
        ],
        [
            IDC_PAUSE_LENGTH_LABEL,
            PAUSE.length,
            IDC_PAUSE_LENGTH_UNIT,
            PAUSE.level,
        ],
    ];
    for (i, (on, [every_label, every, every_unit])) in nudges.into_iter().enumerate() {
        if i > 0 {
            y += ROW + ROW_GAP;
        }
        b.put(on, (PAD, y), (LABEL, ROW), Style::Body);
        b.text(every_label, FIELD, y, 40);
        b.put(every, (FIELD + 48, y), (56, ROW), Style::Body);
        b.text(every_unit, FIELD + 112, y, 28);
        if let Some(&[label, length, unit, level]) = lengths.get(i) {
            b.text(label, FIELD + 156, y, 88);
            b.put(length, (FIELD + 252, y), (52, ROW), Style::Body);
            b.text(unit, FIELD + 312, y, 28);
            let level_w = right - FIELD - 356;
            b.put(level, (FIELD + 356, y), (level_w, ROW), Style::Body);
        }
    }
    y += ROW + theme::SPACE_1;
    let note = (right - FIELD, CAPTION);
    b.put(IDC_WATER_NOTE, (FIELD, y), note, Style::Caption);
    y += CAPTION + SECTION;

    let check = (right - PAD, ROW);
    b.put(IDC_START_WITH_WINDOWS, (PAD, y), check, Style::Body);
    let band = y + ROW + PAD;
    let height = b.footer(band, &[IDOK.0, IDCANCEL.0]);
    Layout {
        width: WIDTH,
        height,
        band,
        places: b.0,
        dot: None,
    }
}

/// A place scaled to `dpi`: each edge scales, so neighbours stay flush.
fn scaled(place: &Place, dpi: u32) -> RECT {
    RECT {
        left: px(place.x, dpi),
        top: px(place.y, dpi),
        right: px(place.x + place.w, dpi),
        bottom: px(place.y + place.h, dpi),
    }
}

/// The focus ring's width at `dpi`.
fn ring_width(dpi: u32) -> i32 {
    px(theme::FOCUS_RING, dpi)
}

/// How far the primary button's window reaches past its fill: the focus ring, then a gap as
/// wide. [`lay_out`] grows the window by it and [`draw_primary`] insets the fill by it, so the
/// fill is always the scaled place, at every DPI.
fn ring_inset(dpi: u32) -> i32 {
    2 * ring_width(dpi)
}

/// `rect` grown by `by` on every side.
fn grown(rect: RECT, by: i32) -> RECT {
    RECT {
        left: rect.left - by,
        top: rect.top - by,
        right: rect.right + by,
        bottom: rect.bottom + by,
    }
}

/// What a dialog keeps for its look: its fonts and the DPI and band it was laid out for.
struct Look {
    fonts: FontCache,
    dpi: u32,
    /// Top of the footer band in client pixels.
    band: i32,
}

impl Look {
    const fn new() -> Look {
        Look {
            fonts: FontCache::new(),
            dpi: 96,
            band: 0,
        }
    }
}

thread_local! {
    /// One per [`Kind`].
    static LOOKS: RefCell<[Look; 2]> = const { RefCell::new([Look::new(), Look::new()]) };
    /// `surface`, `surface-sunken` and `line`, for the life of the app.
    static BRUSHES: [HBRUSH; 3] = [LIGHT.surface, LIGHT.surface_sunken, LIGHT.line]
        // SAFETY: plain call; the brushes are never freed, as controls may hold them.
        .map(|colour| unsafe { CreateSolidBrush(colour) });
    /// The primary buttons and their state.
    static PRIMARIES: RefCell<Vec<(HWND, Primary)>> = const { RefCell::new(Vec::new()) };
}

#[derive(Clone, Copy)]
enum Ground {
    Surface,
    Sunken,
    Line,
}

fn brush(ground: Ground) -> HBRUSH {
    BRUSHES.with(|b| b[ground as usize])
}

/// The font of `style` for `kind` at the DPI it was laid out for.
fn font(kind: Kind, style: Style) -> HFONT {
    LOOKS.with_borrow_mut(|looks| {
        let look = &mut looks[kind as usize];
        look.fonts.at(look.dpi).get(style)
    })
}

fn look_dpi(kind: Kind) -> u32 {
    LOOKS.with_borrow(|looks| looks[kind as usize].dpi)
}

fn look_band(kind: Kind) -> i32 {
    LOOKS.with_borrow(|looks| looks[kind as usize].band)
}

/// From `WM_INITDIALOG`: lay the dialog out for its monitor, centre it there and hook up the
/// primary button.
fn set_up(dialog: HWND, kind: Kind) {
    // SAFETY: `dialog` is the window being initialised.
    unsafe {
        // This module lays out again on a DPI change; the dialog manager must not scale too.
        let _ = SetDialogDpiChangeBehavior(dialog, DDC_DISABLE_ALL, DDC_DISABLE_ALL);
        lay_out(dialog, kind, GetDpiForWindow(dialog));
        centre(dialog);
        SendMessageW(dialog, DM_SETDEFID, Some(WPARAM(IDOK.0 as usize)), None);
        if let Ok(button) = GetDlgItem(Some(dialog), IDOK.0) {
            PRIMARIES.with_borrow_mut(|p| p.push((button, Primary::new())));
            let _ = SetWindowSubclass(button, Some(primary_proc), 0, 0);
        }
    }
}

/// Places, sizes and sets the font of every control for `dpi`, then sizes the window.
fn lay_out(dialog: HWND, kind: Kind, dpi: u32) {
    let layout = kind.layout();
    LOOKS.with_borrow_mut(|looks| {
        let look = &mut looks[kind as usize];
        look.dpi = dpi;
        look.band = px(layout.band, dpi);
    });
    // SAFETY: `dialog` is a live dialog; every control found is its child.
    unsafe {
        for place in &layout.places {
            let Ok(control) = GetDlgItem(Some(dialog), place.id) else {
                continue;
            };
            let font = font(kind, place.style);
            SendMessageW(control, WM_SETFONT, Some(WPARAM(font.0 as usize)), None);
            let mut rect = scaled(place, dpi);
            if place.id == IDOK.0 {
                // The primary button paints its focus ring 2px off its fill, so its window is
                // larger than the fill by the ring and the gap.
                rect = grown(rect, ring_inset(dpi));
            }
            let combo = COMBOS.contains(&place.id);
            let drop = if combo { px(DROP, dpi) } else { 0 };
            let flags = SWP_NOZORDER | SWP_NOACTIVATE;
            let (w, h) = (rect.right - rect.left, rect.bottom - rect.top);
            let _ = SetWindowPos(control, None, rect.left, rect.top, w, h + drop, flags);
            if combo {
                // A combo box takes its height from its font: centre it in the row.
                let mut now = RECT::default();
                let _ = GetWindowRect(control, &mut now);
                let top = rect.top + (h - (now.bottom - now.top)) / 2;
                let _ = SetWindowPos(control, None, rect.left, top, 0, 0, flags | SWP_NOSIZE);
            }
        }
        // Each up-down control takes the width of a scroll bar at this DPI, inside its edit
        // box's place at the right edge; the edit box keeps the rest. UDM_SETBUDDY sizes the
        // up-down for the monitor it was made on, so it is placed after that.
        let spin_w = GetSystemMetricsForDpi(SM_CXVSCROLL, dpi);
        let flags = SWP_NOZORDER | SWP_NOACTIVATE;
        for (spin_id, edit_id) in SPINS {
            let place = layout.places.iter().find(|p| p.id == edit_id);
            let (Ok(spin), Ok(edit), Some(place)) = (
                GetDlgItem(Some(dialog), spin_id),
                GetDlgItem(Some(dialog), edit_id),
                place,
            ) else {
                continue;
            };
            SendMessageW(spin, UDM_SETBUDDY, Some(WPARAM(edit.0 as usize)), None);
            let r = scaled(place, dpi);
            let (w, h) = (r.right - r.left, r.bottom - r.top);
            let _ = SetWindowPos(edit, None, r.left, r.top, w - spin_w, h, flags);
            let _ = SetWindowPos(spin, None, r.right - spin_w, r.top, spin_w, h, flags);
        }
        let style = WINDOW_STYLE(GetWindowLongW(dialog, GWL_STYLE) as u32);
        let ex_style = WINDOW_EX_STYLE(GetWindowLongW(dialog, GWL_EXSTYLE) as u32);
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: px(layout.width, dpi),
            bottom: px(layout.height, dpi),
        };
        let _ = AdjustWindowRectExForDpi(&mut rect, style, false, ex_style, dpi);
        let (w, h) = (rect.right - rect.left, rect.bottom - rect.top);
        let flags = SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE;
        let _ = SetWindowPos(dialog, None, 0, 0, w, h, flags);
        let _ = RedrawWindow(
            Some(dialog),
            None,
            None,
            RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN,
        );
    }
}

/// Centres the dialog in the work area of its monitor.
fn centre(dialog: HWND) {
    // SAFETY: `dialog` is a live window; `info` and `rect` are valid out pointers.
    unsafe {
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let monitor = MonitorFromWindow(dialog, MONITOR_DEFAULTTONEAREST);
        let mut rect = RECT::default();
        if !GetMonitorInfoW(monitor, &mut info).as_bool()
            || GetWindowRect(dialog, &mut rect).is_err()
        {
            return;
        }
        let work = info.rcWork;
        let x = work.left + (work.right - work.left - (rect.right - rect.left)) / 2;
        let y = work.top + (work.bottom - work.top - (rect.bottom - rect.top)) / 2;
        let flags = SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE;
        let _ = SetWindowPos(dialog, None, x, y, 0, 0, flags);
    }
}

/// The messages both dialogs handle the same way for their look. `Some` is the dialog
/// procedure's return value; `None` leaves the message to the dialog's own procedure.
fn look(dialog: HWND, kind: Kind, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<isize> {
    match msg {
        // The ground and the band, also under a themed button's corners, which it asks for
        // with WM_PRINTCLIENT.
        WM_ERASEBKGND | WM_PRINTCLIENT => {
            paint_ground(dialog, kind, HDC(wparam.0 as _));
            if msg == WM_ERASEBKGND {
                let result = WINDOW_LONG_PTR_INDEX(DWLP_MSGRESULT as i32);
                // SAFETY: `dialog` is a live dialog; this is its result for the message.
                unsafe { SetWindowLongPtrW(dialog, result, 1) };
            }
            Some(1)
        }
        WM_CTLCOLORDLG => Some(brush(Ground::Surface).0 as isize),
        WM_CTLCOLORSTATIC | WM_CTLCOLORBTN => {
            let (dc, control) = (HDC(wparam.0 as _), HWND(lparam.0 as _));
            let (ground, bg) = if on_band(dialog, kind, control) {
                (Ground::Sunken, LIGHT.surface_sunken)
            } else {
                (Ground::Surface, LIGHT.surface)
            };
            // SAFETY: `control` is a child of `dialog`.
            let id = unsafe { GetDlgCtrlID(control) };
            let muted = kind
                .layout()
                .places
                .iter()
                .any(|p| p.id == id && p.style == Style::Caption);
            let ink = if muted { LIGHT.ink_muted } else { LIGHT.ink };
            // SAFETY: `dc` is the control's DC for this message.
            unsafe {
                SetTextColor(dc, ink);
                SetBkColor(dc, bg);
            }
            Some(brush(ground).0 as isize)
        }
        WM_DRAWITEM => {
            // SAFETY: for WM_DRAWITEM, lParam points to a DRAWITEMSTRUCT for this call.
            let item = unsafe { &*(lparam.0 as *const DRAWITEMSTRUCT) };
            if item.CtlType != ODT_BUTTON {
                return None;
            }
            draw_primary(kind, item);
            Some(1)
        }
        WM_DPICHANGED => {
            let dpi = (wparam.0 & 0xffff) as u32;
            lay_out(dialog, kind, dpi);
            // SAFETY: for WM_DPICHANGED, lParam points to the suggested window rectangle.
            let at = unsafe { &*(lparam.0 as *const RECT) };
            let flags = SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE;
            // SAFETY: `dialog` is a live dialog.
            let _ = unsafe { SetWindowPos(dialog, None, at.left, at.top, 0, 0, flags) };
            Some(1)
        }
        _ => None,
    }
}

/// Whether `control` sits on the footer band.
fn on_band(dialog: HWND, kind: Kind, control: HWND) -> bool {
    let mut corners = [POINT::default(); 2];
    let mut rect = RECT::default();
    // SAFETY: both are live windows; the out pointers are valid.
    unsafe {
        if GetWindowRect(control, &mut rect).is_err() {
            return false;
        }
        corners[0] = POINT {
            x: rect.left,
            y: rect.top,
        };
        corners[1] = POINT {
            x: rect.right,
            y: rect.bottom,
        };
        MapWindowPoints(None, Some(dialog), &mut corners);
    }
    corners[0].y >= look_band(kind)
}

/// `surface` above the band; the band in `surface-sunken` under a 1px `line` hairline; the
/// wizard's `dot` before its heading.
fn paint_ground(dialog: HWND, kind: Kind, dc: HDC) {
    let mut client = RECT::default();
    // SAFETY: `dialog` is live; `dc` paints its client area.
    unsafe {
        let _ = GetClientRect(dialog, &mut client);
        let band = look_band(kind);
        let hairline = band + px(1, look_dpi(kind));
        let parts = [
            (client.top, band, Ground::Surface),
            (band, hairline, Ground::Line),
            (hairline, client.bottom, Ground::Sunken),
        ];
        for (top, bottom, ground) in parts {
            let rect = RECT {
                top,
                bottom,
                ..client
            };
            FillRect(dc, &rect, brush(ground));
        }
        if let Some((x, y)) = kind.layout().dot {
            let dpi = look_dpi(kind);
            let mark = theme::SIZE_MARK;
            let pen = CreatePen(PS_INSIDEFRAME, 1, LIGHT.dot);
            let fill = CreateSolidBrush(LIGHT.dot);
            let old_pen = SelectObject(dc, pen.into());
            let old_brush = SelectObject(dc, fill.into());
            let (left, top) = (px(x, dpi), px(y, dpi));
            let _ = Ellipse(dc, left, top, px(x + mark, dpi), px(y + mark, dpi));
            SelectObject(dc, old_brush);
            SelectObject(dc, old_pen);
            let _ = DeleteObject(fill.into());
            let _ = DeleteObject(pen.into());
        }
    }
}

// ---- The primary button (DESIGN.md §6 Buttons, §8 Motion) ----

/// The primary button's timer, which repaints it while its fill fades.
const FADE_TIMER: usize = 1;
/// About one frame at 60 Hz.
const FRAME_MS: u32 = 16;

/// The primary fill as a level from `action` (0) to `action-hover` (1), moving over time.
#[derive(Clone, Copy, Debug)]
struct Fade {
    from: f32,
    to: f32,
    start: Instant,
    length: Duration,
}

impl Fade {
    fn at_rest(now: Instant) -> Fade {
        Fade {
            from: 0.0,
            to: 0.0,
            start: now,
            length: Duration::ZERO,
        }
    }

    fn level(&self, now: Instant) -> f32 {
        let gone = now.saturating_duration_since(self.start);
        if gone >= self.length {
            return self.to;
        }
        let t = gone.as_secs_f32() / self.length.as_secs_f32();
        self.from + (self.to - self.from) * t
    }

    fn done(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.start) >= self.length
    }

    /// Heads for `to` from where the fill is now; `whole` is the time for the whole way, so
    /// turning back halfway takes half of it.
    fn toward(&mut self, to: f32, whole: Duration, now: Instant) {
        let from = self.level(now);
        *self = Fade {
            from,
            to,
            start: now,
            length: whole.mul_f32((to - from).abs()),
        };
    }
}

/// A primary button's state: pointer over it, pressed, and its fill.
struct Primary {
    hot: bool,
    pressed: bool,
    fade: Fade,
}

impl Primary {
    fn new() -> Primary {
        Primary {
            hot: false,
            pressed: false,
            fade: Fade::at_rest(Instant::now()),
        }
    }

    /// Fades toward `action-hover` while hot or pressed, else back to `action`: 150 ms in,
    /// 120 ms out, 90 ms for a press; at once when Windows animation is off.
    fn retarget(&mut self, now: Instant, animate: bool) {
        let to = if self.hot || self.pressed { 1.0 } else { 0.0 };
        let whole = if !animate {
            Duration::ZERO
        } else if self.pressed {
            theme::PRESS
        } else if to > self.fade.level(now) {
            theme::HOVER_IN
        } else {
            theme::HOVER_OUT
        };
        self.fade.toward(to, whole, now);
    }
}

/// Runs `f` on the state of primary button `button`.
fn with_primary<R>(button: HWND, f: impl FnOnce(&mut Primary) -> R) -> Option<R> {
    PRIMARIES.with_borrow_mut(|all| {
        all.iter_mut()
            .find(|(hwnd, _)| *hwnd == button)
            .map(|(_, p)| f(p))
    })
}

/// Sets hot or pressed as `change` does; when that changed anything, starts the fade.
fn primary_changed(button: HWND, change: impl FnOnce(&mut Primary) -> bool) {
    let changed = with_primary(button, |p| {
        let changed = change(p);
        if changed {
            p.retarget(Instant::now(), theme::animations_on());
        }
        changed
    });
    if changed == Some(true) {
        // SAFETY: `button` is a live window.
        unsafe {
            SetTimer(Some(button), FADE_TIMER, FRAME_MS, None);
            let _ = InvalidateRect(Some(button), None, false);
        }
    }
}

/// The primary button's subclass: tracks the pointer and repaints while the fill fades.
unsafe extern "system" fn primary_proc(
    button: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _: usize,
    _: usize,
) -> LRESULT {
    match msg {
        WM_MOUSEMOVE => primary_changed(button, |p| {
            let entered = !p.hot;
            if entered {
                let mut track = TRACKMOUSEEVENT {
                    cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: button,
                    dwHoverTime: 0,
                };
                // SAFETY: `track` is a valid TRACKMOUSEEVENT for this call.
                let _ = unsafe { TrackMouseEvent(&mut track) };
                p.hot = true;
            }
            entered
        }),
        WM_MOUSELEAVE => primary_changed(button, |p| std::mem::replace(&mut p.hot, false)),
        WM_TIMER if wparam.0 == FADE_TIMER => {
            let done = with_primary(button, |p| p.fade.done(Instant::now())).unwrap_or(true);
            // SAFETY: `button` is a live window.
            unsafe {
                let _ = InvalidateRect(Some(button), None, false);
                if done {
                    let _ = KillTimer(Some(button), FADE_TIMER);
                }
            }
            return LRESULT(0);
        }
        // WM_DRAWITEM paints every pixel.
        WM_ERASEBKGND => return LRESULT(1),
        WM_NCDESTROY => {
            PRIMARIES.with_borrow_mut(|all| all.retain(|(hwnd, _)| *hwnd != button));
            // SAFETY: removes the subclass this module set.
            let _ = unsafe { RemoveWindowSubclass(button, Some(primary_proc), 0) };
        }
        _ => {}
    }
    // SAFETY: passes the message on down the subclass chain.
    unsafe { DefSubclassProc(button, msg, wparam, lparam) }
}

/// A colour `t` of the way from `a` to `b`.
fn mix(a: COLORREF, b: COLORREF, t: f32) -> COLORREF {
    let channel = |shift: u32| {
        let (x, y) = ((a.0 >> shift) & 0xff, (b.0 >> shift) & 0xff);
        let v = x as f32 + (y as f32 - x as f32) * t.clamp(0.0, 1.0);
        (v.round() as u32) << shift
    };
    COLORREF(channel(0) | channel(8) | channel(16))
}

/// Draws the primary button: the `action` fill at `radius-sm`, its label in `body-strong`
/// `on-action`, and the focus ring 2px off the fill. Off screen first, so it never flickers.
fn draw_primary(kind: Kind, item: &DRAWITEMSTRUCT) {
    let dpi = look_dpi(kind);
    let state = item.itemState.0;
    let pressed = state & ODS_SELECTED.0 != 0;
    let ring = state & ODS_FOCUS.0 != 0 && state & ODS_NOFOCUSRECT.0 == 0;
    // A press starts its fade here, where the button first shows it.
    primary_changed(item.hwndItem, |p| {
        p.pressed != std::mem::replace(&mut p.pressed, pressed)
    });
    let level = with_primary(item.hwndItem, |p| p.fade.level(Instant::now())).unwrap_or(0.0);
    let fill = mix(LIGHT.action, LIGHT.action_hover, level);

    let r = item.rcItem;
    let (w, h) = (r.right - r.left, r.bottom - r.top);
    let width = ring_width(dpi);
    let inset = ring_inset(dpi);
    let radius = px(theme::RADIUS_SM, dpi);
    let mut label = [0u16; 32];
    // SAFETY: every GDI object made here is selected out and freed before return; the DCs and
    // the button are live for the call.
    unsafe {
        let n = GetWindowTextW(item.hwndItem, &mut label) as usize;
        let dc = CreateCompatibleDC(Some(item.hDC));
        let bitmap = CreateCompatibleBitmap(item.hDC, w, h);
        let old_bitmap = SelectObject(dc, bitmap.into());
        let all = RECT {
            left: 0,
            top: 0,
            right: w,
            bottom: h,
        };
        FillRect(dc, &all, brush(Ground::Sunken));

        if ring {
            let pen = CreatePen(PS_INSIDEFRAME, width, LIGHT.focus);
            let old_pen = SelectObject(dc, pen.into());
            let old_brush = SelectObject(dc, GetStockObject(NULL_BRUSH));
            let round = 2 * (radius + inset);
            let _ = RoundRect(dc, 0, 0, w, h, round, round);
            SelectObject(dc, old_brush);
            SelectObject(dc, old_pen);
            let _ = DeleteObject(pen.into());
        }

        let pen = CreatePen(PS_INSIDEFRAME, 1, fill);
        let fill_brush = CreateSolidBrush(fill);
        let old_pen = SelectObject(dc, pen.into());
        let old_brush = SelectObject(dc, fill_brush.into());
        let round = 2 * radius;
        let _ = RoundRect(dc, inset, inset, w - inset, h - inset, round, round);
        SelectObject(dc, old_brush);
        SelectObject(dc, old_pen);
        let _ = DeleteObject(fill_brush.into());
        let _ = DeleteObject(pen.into());

        let old_font = SelectObject(dc, font(kind, Style::BodyStrong).into());
        SetBkMode(dc, TRANSPARENT);
        SetTextColor(dc, LIGHT.on_action);
        // The label moves 1px down while pressed.
        let down = if pressed { px(1, dpi) } else { 0 };
        let mut text = RECT {
            left: inset,
            top: inset + down,
            right: w - inset,
            bottom: h - inset + down,
        };
        let format = DT_CENTER | DT_VCENTER | DT_SINGLELINE;
        DrawTextW(dc, &mut label[..n], &mut text, format);
        SelectObject(dc, old_font);

        let _ = BitBlt(item.hDC, r.left, r.top, w, h, Some(dc), 0, 0, SRCCOPY);
        SelectObject(dc, old_bitmap);
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(dc);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Graphics::Gdi::{
        BI_RGB, BITMAPFILEHEADER, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, DT_CALCRECT, GetDC,
        GetDIBits, ReleaseDC,
    };
    use windows::Win32::Graphics::Gdi::{EnumDisplayMonitors, HMONITOR};
    use windows::Win32::UI::HiDpi::{
        DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForMonitor, MDT_EFFECTIVE_DPI,
        SetThreadDpiAwarenessContext,
    };
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        GetFocus, SetActiveWindow, VK_DOWN, VK_ESCAPE, VK_RETURN, VK_TAB,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        BS_OWNERDRAW, DispatchMessageW, FindWindowExW, GW_CHILD, GW_HWNDNEXT, GetClassNameW,
        GetWindow, HWND_TOPMOST, IsWindow, PM_REMOVE, PRF_CHILDREN, PRF_CLIENT, PRF_ERASEBKGND,
        PRF_NONCLIENT, PeekMessageW, TranslateMessage, UIS_CLEAR, UISF_HIDEFOCUS, WM_CHANGEUISTATE,
        WM_GETFONT, WM_KEYDOWN, WM_PRINT,
    };
    use windows::core::BOOL;

    const SCALES: [u32; 3] = [96, 144, 192];

    fn overlap(a: &Place, b: &Place) -> bool {
        a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
    }

    #[test]
    fn both_dialogs_are_as_wide_so_change_opens_settings_in_place() {
        assert_eq!(wizard_layout().width, settings_layout().width);
    }

    #[test]
    fn controls_keep_apart_inside_the_dialog_and_buttons_sit_on_the_band() {
        for kind in [Kind::Wizard, Kind::Settings] {
            let layout = kind.layout();
            for (i, a) in layout.places.iter().enumerate() {
                assert!(
                    a.x >= PAD && a.x + a.w <= layout.width - PAD,
                    "{kind:?} {a:?}"
                );
                let footer = a.id == IDOK.0 || a.id == IDCANCEL.0;
                if footer {
                    assert!(a.y > layout.band && a.y + a.h < layout.height, "{a:?}");
                    assert!(a.w >= theme::PRIMARY_MIN_WIDTH);
                } else {
                    assert!(a.y >= PAD && a.y + a.h <= layout.band - PAD, "{a:?}");
                }
                for b in &layout.places[i + 1..] {
                    assert!(!overlap(a, b), "{kind:?}: {a:?} overlaps {b:?}");
                }
            }
            let ids: Vec<_> = layout.places.iter().map(|p| p.id).collect();
            let mut unique = ids.clone();
            unique.sort();
            unique.dedup();
            assert_eq!(unique.len(), ids.len(), "{kind:?} places a control twice");
        }
    }

    #[test]
    fn primary_and_cancel_are_right_aligned_and_eight_apart() {
        let layout = settings_layout();
        let at = |id: i32| *layout.places.iter().find(|p| p.id == id).unwrap();
        let (save, cancel) = (at(IDOK.0), at(IDCANCEL.0));
        assert_eq!(cancel.x + cancel.w, layout.width - PAD);
        assert_eq!(cancel.x - (save.x + save.w), theme::SPACE_2);
        let start = *wizard_layout()
            .places
            .iter()
            .find(|p| p.id == IDOK.0)
            .unwrap();
        assert_eq!(start.x + start.w, layout.width - PAD);
    }

    #[test]
    fn numbers_line_drops_the_name() {
        let line = numbers_line(Profile::Balanced);
        assert!(line.starts_with("Stand every 30 min"), "{line}");
        for profile in Profile::ALL {
            assert!(!numbers_line(profile).contains(profile_name(profile)));
        }
    }

    #[test]
    fn primary_fill_is_its_scaled_place_and_ring_and_gap_match_at_125() {
        let dpi = 120;
        let place = *settings_layout()
            .places
            .iter()
            .find(|p| p.id == IDOK.0)
            .unwrap();
        let fill = scaled(&place, dpi);
        assert_eq!((fill.right - fill.left, fill.bottom - fill.top), (110, 40));
        let window = grown(fill, ring_inset(dpi));
        let (w, h) = (window.right - window.left, window.bottom - window.top);
        // draw_primary's fill: the window's own rectangle inset by the same amount.
        let inset = ring_inset(dpi);
        assert_eq!((w - 2 * inset, h - 2 * inset), (110, 40));
        assert_eq!(ring_width(dpi), inset - ring_width(dpi), "ring and gap");
        for dpi in [96, 120, 144, 192] {
            assert_eq!(ring_inset(dpi), 2 * ring_width(dpi));
        }
    }

    #[test]
    fn wizard_dot_sits_space_2_before_the_heading_centred_on_it() {
        let layout = wizard_layout();
        let (x, y) = layout.dot.expect("the wizard has the dot");
        let title = *layout
            .places
            .iter()
            .find(|p| p.id == IDC_WIZARD_TITLE)
            .unwrap();
        assert_eq!(title.x - (x + theme::SIZE_MARK), theme::SPACE_2);
        let line = Style::TitleLg.line_height();
        assert_eq!(2 * y + theme::SIZE_MARK, 2 * title.y + line);
        assert_eq!(x, PAD);
        assert!(settings_layout().dot.is_none());
    }

    #[test]
    fn fade_takes_its_time_and_turns_back_from_where_it_is() {
        let t0 = Instant::now();
        let ms = |n| t0 + Duration::from_millis(n);
        let mut fade = Fade::at_rest(t0);
        fade.toward(1.0, theme::HOVER_IN, t0);
        assert!((fade.level(ms(75)) - 0.5).abs() < 0.01);
        assert!(!fade.done(ms(149)) && fade.done(ms(150)));
        assert_eq!(fade.level(ms(150)), 1.0);
        // Halfway in, the pointer leaves: back over half of the 120 ms.
        let mut fade = Fade::at_rest(t0);
        fade.toward(1.0, theme::HOVER_IN, t0);
        fade.toward(0.0, theme::HOVER_OUT, ms(75));
        assert!(fade.done(ms(135)) && !fade.done(ms(134)));
        assert_eq!(fade.level(ms(135)), 0.0);
    }

    #[test]
    fn press_fades_within_90_ms_and_nothing_fades_with_animation_off() {
        let t0 = Instant::now();
        let mut button = Primary::new();
        button.pressed = true;
        button.retarget(t0, true);
        assert!(button.fade.done(t0 + theme::PRESS));
        button.pressed = false;
        button.retarget(t0 + theme::PRESS, false);
        assert!(button.fade.done(t0 + theme::PRESS));
        assert_eq!(button.fade.level(t0 + theme::PRESS), 0.0);
    }

    #[test]
    fn mix_runs_from_action_to_action_hover() {
        assert_eq!(mix(LIGHT.action, LIGHT.action_hover, 0.0), LIGHT.action);
        assert_eq!(
            mix(LIGHT.action, LIGHT.action_hover, 1.0),
            LIGHT.action_hover
        );
        assert_eq!(
            mix(theme::rgb(0x000000), theme::rgb(0xff0080), 0.5),
            theme::rgb(0x800040)
        );
    }

    /// Opens `kind` hidden, as the app would, without an owner.
    fn create(kind: Kind) -> HWND {
        let form = Form::new(&Default::default());
        let init = Init {
            form: &form,
            start_with_windows: true,
        };
        let picks = Picks::default();
        unsafe {
            let _ = InitCommonControlsEx(&INITCOMMONCONTROLSEX {
                dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
                dwICC: ICC_STANDARD_CLASSES | ICC_DATE_CLASSES | ICC_UPDOWN_CLASS,
            });
            let instance = GetModuleHandleW(None).unwrap();
            let (template, proc, param) = match kind {
                Kind::Wizard => (
                    IDD_WIZARD,
                    wizard_proc as _,
                    &picks as *const Picks as isize,
                ),
                Kind::Settings => (
                    IDD_SETTINGS,
                    settings_proc as _,
                    &init as *const Init as isize,
                ),
            };
            CreateDialogParamW(
                Some(instance.into()),
                resource(template),
                None,
                Some(proc),
                LPARAM(param),
            )
            .expect("the template is embedded")
        }
    }

    fn children(dialog: HWND) -> Vec<HWND> {
        let mut all = Vec::new();
        let mut next = unsafe { GetWindow(dialog, GW_CHILD) };
        while let Ok(child) = next {
            all.push(child);
            next = unsafe { GetWindow(child, GW_HWNDNEXT) };
        }
        all
    }

    /// `control`'s rectangle in `dialog`'s client coordinates.
    fn in_client(dialog: HWND, control: HWND) -> RECT {
        let mut r = RECT::default();
        unsafe { GetWindowRect(control, &mut r).unwrap() };
        let mut corners = [
            POINT {
                x: r.left,
                y: r.top,
            },
            POINT {
                x: r.right,
                y: r.bottom,
            },
        ];
        unsafe { MapWindowPoints(None, Some(dialog), &mut corners) };
        RECT {
            left: corners[0].x,
            top: corners[0].y,
            right: corners[1].x,
            bottom: corners[1].y,
        }
    }

    fn class(control: HWND) -> String {
        let mut name = [0u16; 64];
        let n = unsafe { GetClassNameW(control, &mut name) } as usize;
        String::from_utf16_lossy(&name[..n])
    }

    /// The size of `control`'s text in its own font.
    fn text_size(control: HWND) -> Option<(i32, i32)> {
        let mut text = [0u16; 128];
        let n = unsafe { GetWindowTextW(control, &mut text) } as usize;
        if n == 0 {
            return None;
        }
        unsafe {
            let font = SendMessageW(control, WM_GETFONT, None, None);
            let dc = GetDC(None);
            let old = SelectObject(dc, HFONT(font.0 as _).into());
            let mut rect = RECT::default();
            DrawTextW(dc, &mut text[..n], &mut rect, DT_CALCRECT | DT_SINGLELINE);
            SelectObject(dc, old);
            ReleaseDC(None, dc);
            Some((rect.right - rect.left, rect.bottom - rect.top))
        }
    }

    #[test]
    fn every_control_is_placed_and_its_text_fits_at_100_150_and_200() {
        let mut misfits = Vec::new();
        for kind in [Kind::Wizard, Kind::Settings] {
            let dialog = create(kind);
            let layout = kind.layout();
            let spins = SPINS.map(|(spin, _)| spin);
            // At the monitor's own DPI the client area is exactly the layout's size. (At any
            // other, the frame Windows draws stays at the monitor's DPI.)
            let real = unsafe { GetDpiForWindow(dialog) };
            let mut client = RECT::default();
            unsafe { GetClientRect(dialog, &mut client).unwrap() };
            assert_eq!(client.right, px(layout.width, real), "{kind:?} at {real}");
            assert_eq!(client.bottom, px(layout.height, real), "{kind:?} at {real}");
            for dpi in SCALES {
                lay_out(dialog, kind, dpi);
                for (spin_id, edit_id) in SPINS {
                    let Ok(spin) = (unsafe { GetDlgItem(Some(dialog), spin_id) }) else {
                        continue;
                    };
                    let edit = unsafe { GetDlgItem(Some(dialog), edit_id).unwrap() };
                    let (s, e) = (in_client(dialog, spin), in_client(dialog, edit));
                    let place = layout.places.iter().find(|p| p.id == edit_id).unwrap();
                    let want = unsafe { GetSystemMetricsForDpi(SM_CXVSCROLL, dpi) };
                    let what = format!("up-down of {edit_id} at {dpi}");
                    assert_eq!(s.right - s.left, want, "{what}: width");
                    assert_eq!(s.right, scaled(place, dpi).right, "{what}: right edge");
                    assert_eq!(e.right, s.left, "{what}: edit box ends where it starts");
                }
                for control in children(dialog) {
                    let id = unsafe { GetDlgCtrlID(control) };
                    if spins.contains(&id) {
                        continue;
                    }
                    let place = layout.places.iter().find(|p| p.id == id);
                    let place = place.unwrap_or_else(|| panic!("{kind:?}: {id} is not placed"));
                    let font = unsafe { SendMessageW(control, WM_GETFONT, None, None) };
                    assert_eq!(font.0 as usize, super::font(kind, place.style).0 as usize);
                    let Some((w, h)) = text_size(control) else {
                        continue;
                    };
                    let rect = scaled(place, dpi);
                    let mut room = rect.right - rect.left;
                    if class(control) == "Button" {
                        let style = unsafe { GetWindowLongW(control, GWL_STYLE) } & 0xf;
                        // A checkbox or radio button: its text starts after the glyph. A push
                        // button: its label keeps `space-3` each side.
                        room -= match style {
                            3 | 9 => px(GLYPH, dpi),
                            _ => 2 * px(theme::SPACE_3, dpi),
                        };
                    }
                    let tall = rect.bottom - rect.top;
                    if w > room || h > tall {
                        misfits.push(format!("{kind:?} {id} at {dpi}: {w}x{h} in {room}x{tall}"));
                    }
                }
            }
            unsafe { DestroyWindow(dialog).unwrap() };
        }
        assert!(
            misfits.is_empty(),
            "text does not fit:
{}",
            misfits.join(
                "
"
            )
        );
    }

    /// Presses `vk` on the focused control, the way the app's message loop hands it over.
    fn press(dialog: HWND, vk: u16) {
        unsafe {
            let msg = MSG {
                hwnd: GetFocus(),
                message: WM_KEYDOWN,
                wParam: WPARAM(vk as usize),
                ..Default::default()
            };
            assert!(IsDialogMessageW(dialog, &msg).as_bool());
        }
    }

    fn focused() -> i32 {
        unsafe { GetDlgCtrlID(GetFocus()) }
    }

    /// Shows `dialog` and makes it the active window of this thread, as a user's click would.
    fn activate(dialog: HWND) {
        unsafe {
            let _ = ShowWindow(dialog, SW_SHOW);
            let _ = SetForegroundWindow(dialog);
            let _ = SetActiveWindow(dialog);
        }
    }

    #[test]
    #[ignore = "shows the dialogs"]
    fn tab_runs_in_the_order_the_page_reads() {
        let settings = create(Kind::Settings);
        activate(settings);
        let days = (0..7).map(|i| IDC_DAY + i);
        let rows = [STAND, PAUSE]
            .iter()
            .flat_map(|r| [r.on, r.every, r.length, r.level])
            .collect::<Vec<_>>();
        let want: Vec<i32> = days
            .chain([IDC_FROM, IDC_TO, IDC_PROFILE])
            .chain(rows)
            .chain([IDC_WATER_ON, IDC_WATER_EVERY, IDC_START_WITH_WINDOWS])
            .chain([IDOK.0, IDCANCEL.0, IDC_DAY])
            .collect();
        let mut got = vec![focused()];
        for _ in 1..want.len() {
            press(settings, VK_TAB.0);
            got.push(focused());
        }
        assert_eq!(got, want);
        // Focus on Cancel made it the default for a moment; Save stays owner-drawn.
        let save = unsafe { GetDlgItem(Some(settings), IDOK.0).unwrap() };
        let kind = unsafe { GetWindowLongW(save, GWL_STYLE) } & 0xf;
        assert_eq!(kind, BS_OWNERDRAW);
        unsafe { DestroyWindow(settings).unwrap() };

        let wizard = create(Kind::Wizard);
        activate(wizard);
        let balanced = IDC_WIZARD_PROFILE + 1;
        let mut got = vec![focused()];
        for _ in 0..4 {
            press(wizard, VK_TAB.0);
            got.push(focused());
        }
        let want = [
            balanced,
            IDC_WIZARD_CHANGE,
            IDC_WIZARD_START_WITH_WINDOWS,
            IDOK.0,
            balanced,
        ];
        assert_eq!(got, want);
        // The arrow keys move among the three profiles only.
        press(wizard, VK_DOWN.0);
        assert_eq!(focused(), IDC_WIZARD_PROFILE + 2);
        press(wizard, VK_DOWN.0);
        assert_eq!(focused(), IDC_WIZARD_PROFILE);
        unsafe { DestroyWindow(wizard).unwrap() };
    }

    #[test]
    #[ignore = "shows the dialogs"]
    fn enter_presses_the_primary_and_esc_cancels() {
        // Enter in an edit box saves.
        let settings = create(Kind::Settings);
        activate(settings);
        OPEN.set(Some(settings));
        focus(settings, STAND.every);
        press(settings, VK_RETURN.0);
        assert!(take_saved().is_some(), "Enter saved");
        assert!(!unsafe { IsWindow(Some(settings)) }.as_bool());

        // Esc cancels: nothing saved, the window gone.
        let settings = create(Kind::Settings);
        activate(settings);
        OPEN.set(Some(settings));
        press(settings, VK_ESCAPE.0);
        assert!(take_saved().is_none(), "Esc saved nothing");
        assert!(!unsafe { IsWindow(Some(settings)) }.as_bool());

        // Enter on the wizard starts; Esc accepts too.
        for vk in [VK_RETURN, VK_ESCAPE] {
            let wizard = create(Kind::Wizard);
            activate(wizard);
            press(wizard, vk.0);
            assert!(
                matches!(take_wizard(), Some(WizardDone::Start(_))),
                "{vk:?}"
            );
        }
    }

    /// Closes the next "breakpoint settings" message box that is not `settings` itself.
    fn close_message_box(settings: HWND) -> std::thread::JoinHandle<()> {
        let settings = settings.0 as isize;
        std::thread::spawn(move || {
            for _ in 0..200 {
                std::thread::sleep(Duration::from_millis(25));
                let mut after = None;
                while let Ok(found) =
                    unsafe { FindWindowExW(None, after, w!("#32770"), w!("breakpoint settings")) }
                {
                    if found.0 as isize != settings {
                        unsafe { PostMessageW(Some(found), WM_CLOSE, WPARAM(0), LPARAM(0)) }
                            .unwrap();
                        return;
                    }
                    after = Some(found);
                }
            }
            panic!("no message box");
        })
    }

    #[test]
    #[ignore = "shows the dialogs and message boxes"]
    fn every_refused_save_focuses_its_field() {
        let set = |d: HWND, id: i32, text: &str| set_text(d, id, text);
        let cases: [(Field, &dyn Fn(HWND)); 7] = [
            (Field::WorkDays, &|d| {
                (0..7).for_each(|i| check(d, IDC_DAY + i, false))
            }),
            (Field::WorkHoursEnd, &|d| {
                set_time(d, IDC_TO, TimeOfDay::new(7, 0).unwrap())
            }),
            (Field::StandEvery, &|d| set(d, STAND.every, "5")),
            (Field::StandLength, &|d| set(d, STAND.length, "9")),
            (Field::PauseEvery, &|d| set(d, PAUSE.every, "999")),
            (Field::PauseLength, &|d| set(d, PAUSE.length, "")),
            (Field::WaterEvery, &|d| set(d, IDC_WATER_EVERY, "1")),
        ];
        for (field, spoil) in cases {
            let settings = create(Kind::Settings);
            activate(settings);
            OPEN.set(Some(settings));
            spoil(settings);
            focus(settings, IDC_START_WITH_WINDOWS);
            let closer = close_message_box(settings);
            press(settings, VK_RETURN.0);
            closer.join().unwrap();
            assert_eq!(focused(), control_of(field), "{field:?}");
            assert!(take_saved().is_none());
            unsafe { DestroyWindow(settings).unwrap() };
        }
    }

    /// The monitors' work areas and DPIs.
    fn monitors() -> Vec<(RECT, u32)> {
        unsafe extern "system" fn found(
            monitor: HMONITOR,
            _: HDC,
            _: *mut RECT,
            all: LPARAM,
        ) -> BOOL {
            unsafe {
                let mut info = MONITORINFO {
                    cbSize: size_of::<MONITORINFO>() as u32,
                    ..Default::default()
                };
                let _ = GetMonitorInfoW(monitor, &mut info);
                let (mut x, mut y) = (96, 96);
                let _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut x, &mut y);
                (*(all.0 as *mut Vec<(RECT, u32)>)).push((info.rcWork, x));
            }
            BOOL(1)
        }
        let mut all = Vec::new();
        unsafe {
            let _ = EnumDisplayMonitors(None, None, Some(found), LPARAM(&mut all as *mut _ as _));
        }
        all
    }

    #[test]
    #[ignore = "shows the dialogs; needs two monitors at different scales"]
    fn moving_to_a_monitor_at_another_scale_lays_out_again() {
        unsafe {
            SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        }
        for kind in [Kind::Wizard, Kind::Settings] {
            let dialog = create(kind);
            activate(dialog);
            let first = unsafe { GetDpiForWindow(dialog) };
            let Some((work, dpi)) = monitors().into_iter().find(|&(_, dpi)| dpi != first) else {
                eprintln!("every monitor is at {first} DPI");
                return;
            };
            let flags = SWP_NOSIZE | SWP_NOZORDER;
            unsafe { SetWindowPos(dialog, None, work.left + 40, work.top + 40, 0, 0, flags) }
                .unwrap();
            pump(300);
            assert_eq!(unsafe { GetDpiForWindow(dialog) }, dpi);
            assert_eq!(look_dpi(kind), dpi, "{kind:?} laid out again");
            let layout = kind.layout();
            let mut client = RECT::default();
            unsafe { GetClientRect(dialog, &mut client).unwrap() };
            assert_eq!(
                (client.right, client.bottom),
                (px(layout.width, dpi), px(layout.height, dpi)),
                "{kind:?} at {dpi}"
            );
            let want = unsafe { GetSystemMetricsForDpi(SM_CXVSCROLL, dpi) };
            for (spin, _) in SPINS {
                if let Ok(spin) = unsafe { GetDlgItem(Some(dialog), spin) } {
                    let r = in_client(dialog, spin);
                    assert_eq!(r.right - r.left, want, "up-down at {dpi} after the move");
                }
            }
            if let Ok(folder) = std::env::var("BP_SHOTS") {
                let path = std::path::Path::new(&folder).join(format!("{kind:?}-moved-{dpi}.bmp"));
                shot(dialog, &path);
            }
            unsafe { DestroyWindow(dialog).unwrap() };
        }
    }

    /// Writes `dialog`'s window, as on screen, to a 32-bit BMP.
    fn shot(dialog: HWND, path: &std::path::Path) {
        unsafe {
            // The dialog draws itself into the bitmap: no screen capture, so no other window
            // can show up in the file. With its frame, so the children land where they are.
            let mut r = RECT::default();
            GetWindowRect(dialog, &mut r).unwrap();
            let (w, h) = (r.right - r.left, r.bottom - r.top);
            let screen = GetDC(None);
            let dc = CreateCompatibleDC(Some(screen));
            let bitmap = CreateCompatibleBitmap(screen, w, h);
            let old = SelectObject(dc, bitmap.into());
            let what = (PRF_NONCLIENT | PRF_CLIENT | PRF_CHILDREN | PRF_ERASEBKGND) as isize;
            SendMessageW(
                dialog,
                WM_PRINT,
                Some(WPARAM(dc.0 as usize)),
                Some(LPARAM(what)),
            );
            SelectObject(dc, old);
            let mut info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w,
                    biHeight: -h,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut pixels = vec![0u8; (w * h * 4) as usize];
            GetDIBits(
                dc,
                bitmap,
                0,
                h as u32,
                Some(pixels.as_mut_ptr() as _),
                &mut info,
                DIB_RGB_COLORS,
            );
            let _ = DeleteObject(bitmap.into());
            let _ = DeleteDC(dc);
            ReleaseDC(None, screen);
            let offset = (size_of::<BITMAPFILEHEADER>() + size_of::<BITMAPINFOHEADER>()) as u32;
            let mut file = Vec::new();
            file.extend_from_slice(b"BM");
            file.extend_from_slice(&(offset + pixels.len() as u32).to_le_bytes());
            file.extend_from_slice(&0u32.to_le_bytes());
            file.extend_from_slice(&offset.to_le_bytes());
            let header = std::slice::from_raw_parts(
                &info.bmiHeader as *const BITMAPINFOHEADER as *const u8,
                size_of::<BITMAPINFOHEADER>(),
            );
            file.extend_from_slice(header);
            file.extend_from_slice(&pixels);
            std::fs::write(path, file).unwrap();
        }
    }

    fn pump(for_ms: u64) {
        let end = Instant::now() + Duration::from_millis(for_ms);
        let mut msg = MSG::default();
        while Instant::now() < end {
            unsafe {
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// For a look by eye: `BP_SHOTS=<folder> cargo test shots -- --ignored` writes each dialog
    /// at 100, 125, 150 and 200 % to BMPs there, the primary button focused, then hot.
    /// Text and layout follow the DPI asked for; native glyphs keep the monitor's.
    #[test]
    #[ignore]
    fn shots() {
        let Ok(folder) = std::env::var("BP_SHOTS") else {
            eprintln!("set BP_SHOTS to a folder for the shots");
            return;
        };
        let folder = std::path::PathBuf::from(folder);
        unsafe {
            SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        }
        for kind in [Kind::Wizard, Kind::Settings] {
            for dpi in [96, 120, 144, 192] {
                let dialog = create(kind);
                lay_out(dialog, kind, dpi);
                unsafe {
                    let flags = SWP_NOSIZE | SWP_NOACTIVATE;
                    SetWindowPos(dialog, Some(HWND_TOPMOST), 0, 0, 0, 0, flags).unwrap();
                    let _ = ShowWindow(dialog, SW_SHOW);
                    let _ = SetForegroundWindow(dialog);
                    focus(dialog, IDOK.0);
                    let cues = (UISF_HIDEFOCUS << 16) | UIS_CLEAR;
                    SendMessageW(dialog, WM_CHANGEUISTATE, Some(WPARAM(cues as usize)), None);
                }
                pump(400);
                shot(dialog, &folder.join(format!("{kind:?}-{dpi}.bmp")));
                let button = unsafe { GetDlgItem(Some(dialog), IDOK.0).unwrap() };
                primary_changed(button, |p| !std::mem::replace(&mut p.hot, true));
                pump(400);
                shot(dialog, &folder.join(format!("{kind:?}-{dpi}-hot.bmp")));
                unsafe { DestroyWindow(dialog).unwrap() };
            }
        }
    }
}
