//! The first-run wizard and settings window DIALOGEX procedures.
//!
//! So far the settings window (SPEC §9.2): a modeless dialog from the `IDD_SETTINGS` template
//! that `build.rs` embeds, at most one at a time, run by the app's own message loop through
//! [`is_dialog_message`] so nudges keep firing while it is open. The dialog copies its controls
//! into a [`Form`] and back; the core decides the profile shown, what picking one fills in and
//! whether a Save is valid. A valid Save closes the window and posts [`WM_SETTINGS_SAVED`] to the
//! owner, which collects it with [`take_saved`] and applies it from its own window procedure.

use std::cell::{Cell, RefCell};

use breakpoint::core::config::{Enforcement, Profile};
use breakpoint::core::settings::{AwayRow, Field, Form, Settings};
use breakpoint::core::time::{TimeOfDay, Weekday};
use windows::Win32::Foundation::{HWND, LPARAM, SYSTEMTIME, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::UI::Controls::{
    BST_CHECKED, BST_UNCHECKED, CheckDlgButton, DTM_GETSYSTEMTIME, DTM_SETFORMATW,
    DTM_SETSYSTEMTIME, EM_SETLIMITTEXT, GDT_VALID, ICC_DATE_CLASSES, ICC_STANDARD_CLASSES,
    ICC_UPDOWN_CLASS, INITCOMMONCONTROLSEX, InitCommonControlsEx, IsDlgButtonChecked,
    UDM_SETRANGE32,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CB_ADDSTRING, CB_DELETESTRING, CB_GETCOUNT, CB_GETCURSEL, CB_SETCURSEL, CBN_SELCHANGE,
    CreateDialogParamW, DestroyWindow, EN_CHANGE, GetDlgItem, GetDlgItemTextW, GetSystemMetrics,
    ICON_BIG, ICON_SMALL, IDCANCEL, IDOK, IMAGE_ICON, IsDialogMessageW, IsIconic, LR_DEFAULTCOLOR,
    LoadImageW, MB_ICONWARNING, MB_OK, MSG, MessageBoxW, PostMessageW, SM_CXICON, SM_CXSMICON,
    SM_CYICON, SM_CYSMICON, SW_RESTORE, SW_SHOW, SendDlgItemMessageW, SendMessageW,
    SetDlgItemTextW, SetForegroundWindow, ShowWindow, WM_APP, WM_CLOSE, WM_COMMAND, WM_DESTROY,
    WM_INITDIALOG, WM_NEXTDLGCTL, WM_SETICON,
};
use windows::core::{HSTRING, PCWSTR, w};

mod ids {
    include!("settings_ids.rs");
}
use ids::*;

/// Asks the main window to open Settings, from a second launch (SPEC §11).
pub const WM_OPEN_SETTINGS: u32 = WM_APP + 3;
/// Posted to the owner when a valid Save closed the settings window; see [`take_saved`].
pub const WM_SETTINGS_SAVED: u32 = WM_APP + 4;

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
/// (SPEC §9.2). `owner` receives [`WM_SETTINGS_SAVED`].
pub fn open_settings(owner: HWND, form: &Form, start_with_windows: bool) {
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
        let _ = ShowWindow(dialog, SW_SHOW);
        let _ = SetForegroundWindow(dialog);
    }
}

/// Lets an open dialog handle `msg` (Tab, Enter, Esc, arrows); true if it did, and then the
/// message must not be translated or dispatched again.
pub fn is_dialog_message(msg: &MSG) -> bool {
    // SAFETY: `dialog` is the live settings window; `msg` is a valid MSG.
    OPEN.get()
        .is_some_and(|dialog| unsafe { IsDialogMessageW(dialog, msg) }.as_bool())
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
    // SAFETY: `dialog` is the live settings window.
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
    match msg {
        WM_INITDIALOG => {
            // SAFETY: `open_settings` passes a pointer to an `Init` that outlives this call.
            let init = unsafe { &*(lparam.0 as *const Init) };
            init_controls(dialog);
            write_form(dialog, init.form);
            check(dialog, IDC_START_WITH_WINDOWS, init.start_with_windows);
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
                (id, _) if id == IDCANCEL.0 => close(dialog),
                (IDC_PROFILE, CBN_SELCHANGE) if edited => pick_profile(dialog),
                (_, EN_CHANGE | CBN_SELCHANGE) if edited => show_profile(dialog),
                _ => return 0,
            }
            1
        }
        // X discards without asking, as Cancel does.
        WM_CLOSE => {
            close(dialog);
            1
        }
        WM_DESTROY => {
            OPEN.set(None);
            0
        }
        _ => 0,
    }
}

fn init_controls(dialog: HWND) {
    // SAFETY: `dialog` is the window being initialised; every string is static.
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
                if let Ok(control) = GetDlgItem(Some(dialog), control_of(invalid.field)) {
                    // Moves the focus the dialog's way, which also selects an edit box's text.
                    SendMessageW(
                        dialog,
                        WM_NEXTDLGCTL,
                        Some(WPARAM(control.0 as usize)),
                        Some(LPARAM(1)),
                    );
                }
            }
        }
        Ok(settings) => {
            SAVED.set(Some(Saved {
                settings,
                start_with_windows: checked(dialog, IDC_START_WITH_WINDOWS),
            }));
            if let Some(owner) = OWNER.get() {
                // SAFETY: `owner` is the app's main window, alive as long as the app.
                let _ =
                    unsafe { PostMessageW(Some(owner), WM_SETTINGS_SAVED, WPARAM(0), LPARAM(0)) };
            }
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

fn close(dialog: HWND) {
    // SAFETY: `dialog` is the live settings window; WM_DESTROY forgets it.
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
