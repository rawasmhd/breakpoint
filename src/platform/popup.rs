//! The nudge popup window.
//!
//! One window for the life of the app, hidden between occurrences (SPEC §6: at most one popup on
//! screen). It sits bottom-right of the primary monitor's work area, just above the tray, always
//! on top, and never takes focus: `WS_EX_NOACTIVATE`, `SW_SHOWNOACTIVATE` and `MA_NOACTIVATE`.
//! It carries the prompt's title and line, a live away countdown the app refreshes every second,
//! and the Snooze 5 min and Skip buttons (Skip only after the snooze). Button presses are posted to the owner window as messages, so the app reacts from its own
//! window procedure rather than from inside this one.

use breakpoint::core::scheduler::Prompt;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    COLOR_WINDOW, CreateFontIndirectW, DeleteObject, FW_BOLD, GetMonitorInfoW, GetSysColor,
    GetSysColorBrush, HDC, HFONT, HMONITOR, MONITOR_DEFAULTTOPRIMARY, MONITORINFO,
    MonitorFromPoint, SetBkColor,
};
use windows::Win32::Media::Audio::{PlaySoundW, SND_ALIAS, SND_ASYNC};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{
    AdjustWindowRectExForDpi, GetDpiForMonitor, MDT_EFFECTIVE_DPI, SystemParametersInfoForDpi,
};
use windows::Win32::UI::WindowsAndMessaging::{
    BN_CLICKED, BS_PUSHBUTTON, CreateWindowExW, DefWindowProcW, GWLP_USERDATA, GetDlgItem,
    GetWindowLongPtrW, HMENU, HWND_TOPMOST, IDC_ARROW, IsWindowVisible, LoadCursorW, MA_NOACTIVATE,
    NONCLIENTMETRICSW, PostMessageW, RegisterClassW, SPI_GETNONCLIENTMETRICS, SW_HIDE, SW_SHOW,
    SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SWP_SHOWWINDOW,
    SendMessageW, SetWindowLongPtrW, SetWindowPos, SetWindowTextW, ShowWindow, WINDOW_EX_STYLE,
    WINDOW_STYLE, WM_APP, WM_CLOSE, WM_COMMAND, WM_CTLCOLORSTATIC, WM_DPICHANGED, WM_GETFONT,
    WM_MOUSEACTIVATE, WM_SETFONT, WNDCLASSW, WS_CAPTION, WS_CHILD, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP, WS_SYSMENU, WS_TABSTOP, WS_VISIBLE,
};
use windows::core::{HSTRING, Result, w};

/// Posted to the owner window when the user presses Skip.
pub const WM_POPUP_SKIP: u32 = WM_APP + 1;
/// Posted to the owner window when the user presses Snooze 5 min.
pub const WM_POPUP_SNOOZE: u32 = WM_APP + 2;

const ID_TITLE: i32 = 1;
const ID_LINE: i32 = 2;
const ID_SKIP: i32 = 3;
const ID_COUNTDOWN: i32 = 4;
const ID_SNOOZE: i32 = 5;
const ID_NOTICE: i32 = 6;

/// The bad-config notice (SPEC §10.1).
const NOTICE: &str = "breakpoint couldn't read config.json and is using the defaults. \
                      Saving from Settings will replace the file.";

const STYLE: WINDOW_STYLE = WINDOW_STYLE(WS_POPUP.0 | WS_CAPTION.0 | WS_SYSMENU.0);
const EX_STYLE: WINDOW_EX_STYLE =
    WINDOW_EX_STYLE(WS_EX_TOPMOST.0 | WS_EX_TOOLWINDOW.0 | WS_EX_NOACTIVATE.0);

/// Layout at 96 DPI, scaled for the monitor: the client size, then each child's rectangle.
const CLIENT: (i32, i32) = (320, 148);
const CHILDREN: [(i32, [i32; 4]); 6] = [
    (ID_TITLE, [14, 12, 292, 24]),
    (ID_LINE, [14, 40, 292, 36]),
    (ID_COUNTDOWN, [14, 80, 292, 20]),
    (ID_SNOOZE, [134, 108, 90, 28]),
    (ID_SKIP, [230, 108, 76, 28]),
    (ID_NOTICE, [14, 12, 292, 124]),
];
/// The children that make up a nudge's popup; the notice hides them all.
const PROMPT: [i32; 5] = [ID_TITLE, ID_LINE, ID_COUNTDOWN, ID_SNOOZE, ID_SKIP];
/// The gap between the popup and the work area's bottom-right corner, at 96 DPI.
const MARGIN: i32 = 12;

pub struct Popup {
    hwnd: HWND,
}

impl Popup {
    /// Creates the popup hidden; `owner` receives its button presses as `WM_POPUP_*` messages.
    pub fn new(owner: HWND) -> Result<Self> {
        // SAFETY: plain Win32 window creation; every handle passed is one created just before.
        unsafe {
            let instance = GetModuleHandleW(None)?.into();
            let class = w!("breakpoint.popup");
            RegisterClassW(&WNDCLASSW {
                lpfnWndProc: Some(wndproc),
                hInstance: instance,
                lpszClassName: class,
                hCursor: LoadCursorW(None, IDC_ARROW)?,
                hbrBackground: GetSysColorBrush(COLOR_WINDOW),
                ..Default::default()
            });
            // Created at the primary monitor's origin so it starts with that monitor's DPI.
            let hwnd = CreateWindowExW(
                EX_STYLE,
                class,
                w!("breakpoint"),
                STYLE,
                0,
                0,
                0,
                0,
                None,
                None,
                Some(instance),
                None,
            )?;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, owner.0 as isize);

            let button = WINDOW_STYLE(BS_PUSHBUTTON as u32 | WS_TABSTOP.0);
            for (id, class, text, style) in [
                (ID_TITLE, w!("STATIC"), w!(""), WINDOW_STYLE(0)),
                (ID_LINE, w!("STATIC"), w!(""), WINDOW_STYLE(0)),
                (ID_COUNTDOWN, w!("STATIC"), w!(""), WINDOW_STYLE(0)),
                (ID_SNOOZE, w!("BUTTON"), w!("Snooze 5 min"), button),
                (ID_SKIP, w!("BUTTON"), w!("Skip"), button),
                (ID_NOTICE, w!("STATIC"), w!(""), WINDOW_STYLE(0)),
            ] {
                CreateWindowExW(
                    WINDOW_EX_STYLE(0),
                    class,
                    text,
                    WS_CHILD | WS_VISIBLE | style,
                    0,
                    0,
                    0,
                    0,
                    Some(hwnd),
                    Some(HMENU(id as isize as _)),
                    Some(instance),
                    None,
                )?;
            }
            Ok(Self { hwnd })
        }
    }

    /// Shows `prompt` on top without taking focus, with the default notification sound.
    pub fn show(&self, prompt: &Prompt, countdown: &str) {
        // SAFETY: `self.hwnd` is the live popup window; the strings outlive the calls.
        unsafe {
            set_text(self.hwnd, ID_TITLE, prompt.title);
            set_text(self.hwnd, ID_LINE, &prompt.line);
            set_text(self.hwnd, ID_COUNTDOWN, countdown);
            show_only(self.hwnd, &PROMPT);
            if let Ok(snooze) = GetDlgItem(Some(self.hwnd), ID_SNOOZE) {
                let _ = ShowWindow(snooze, if prompt.snooze { SW_SHOW } else { SW_HIDE });
            }
            self.raise();
            let _ = PlaySoundW(w!("Notification.Default"), None, SND_ALIAS | SND_ASYNC);
        }
    }

    /// Shows the bad-config notice in the popup's place: no buttons, silent, until closed with
    /// X. A nudge shown later replaces it.
    pub fn notice(&self) {
        // SAFETY: `self.hwnd` is the live popup window; the string outlives the call.
        unsafe {
            set_text(self.hwnd, ID_NOTICE, NOTICE);
            show_only(self.hwnd, &[ID_NOTICE]);
            self.raise();
        }
    }

    /// Lays the popup out afresh and puts it on top without activating it.
    unsafe fn raise(&self) {
        unsafe {
            // The work area or the scale may have changed while hidden.
            layout(self.hwnd, primary_dpi());
            let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
            let _ = SetWindowPos(
                self.hwnd,
                Some(HWND_TOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
        }
    }

    /// Replaces the countdown line, e.g. "Away 1:59 more and it counts".
    pub fn set_countdown(&self, text: &str) {
        // SAFETY: `self.hwnd` is the live popup window; the string outlives the call.
        unsafe { set_text(self.hwnd, ID_COUNTDOWN, text) }
    }

    pub fn is_visible(&self) -> bool {
        // SAFETY: no pointers.
        unsafe { IsWindowVisible(self.hwnd).as_bool() }
    }

    pub fn hide(&self) {
        // SAFETY: `self.hwnd` is the live popup window.
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
    }
}

unsafe fn set_text(hwnd: HWND, id: i32, text: &str) {
    unsafe {
        if let Ok(child) = GetDlgItem(Some(hwnd), id) {
            let _ = SetWindowTextW(child, &HSTRING::from(text));
        }
    }
}

/// Shows the children in `ids` and hides the rest.
unsafe fn show_only(hwnd: HWND, ids: &[i32]) {
    unsafe {
        for (id, _) in CHILDREN {
            if let Ok(child) = GetDlgItem(Some(hwnd), id) {
                let _ = ShowWindow(child, if ids.contains(&id) { SW_SHOW } else { SW_HIDE });
            }
        }
    }
}

fn primary() -> HMONITOR {
    // SAFETY: no pointers; the primary monitor always contains the origin.
    unsafe { MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY) }
}

fn primary_dpi() -> u32 {
    let (mut x, mut y) = (96, 96);
    // SAFETY: `x` and `y` are valid out-pointers for the call.
    let _ = unsafe { GetDpiForMonitor(primary(), MDT_EFFECTIVE_DPI, &mut x, &mut y) };
    x
}

/// Sizes the fonts, the children and the window for `dpi` and moves it bottom-right of the
/// primary monitor's work area.
unsafe fn layout(hwnd: HWND, dpi: u32) {
    let px = |v: i32| v * dpi as i32 / 96;
    unsafe {
        // The system message font at this DPI, and a larger bold copy for the title.
        let mut metrics = NONCLIENTMETRICSW {
            cbSize: size_of::<NONCLIENTMETRICSW>() as u32,
            ..Default::default()
        };
        let _ = SystemParametersInfoForDpi(
            SPI_GETNONCLIENTMETRICS.0,
            metrics.cbSize,
            Some(&mut metrics as *mut _ as _),
            0,
            dpi,
        );
        let body = CreateFontIndirectW(&metrics.lfMessageFont);
        let mut bold = metrics.lfMessageFont;
        bold.lfWeight = FW_BOLD.0 as i32;
        bold.lfHeight = bold.lfHeight * 6 / 5;
        let title = CreateFontIndirectW(&bold);

        // Fonts from the last layout, freed once nothing uses them.
        let mut old = Vec::new();
        for (id, [x, y, w, h]) in CHILDREN {
            let Ok(child) = GetDlgItem(Some(hwnd), id) else {
                continue;
            };
            let font = if id == ID_TITLE { title } else { body };
            let previous = HFONT(SendMessageW(child, WM_GETFONT, None, None).0 as _);
            if !previous.is_invalid() && !old.contains(&previous) {
                old.push(previous);
            }
            SendMessageW(
                child,
                WM_SETFONT,
                Some(WPARAM(font.0 as usize)),
                Some(LPARAM(1)),
            );
            let _ = SetWindowPos(
                child,
                None,
                px(x),
                px(y),
                px(w),
                px(h),
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
        for font in old {
            let _ = DeleteObject(font.into());
        }

        let mut rect = RECT {
            left: 0,
            top: 0,
            right: px(CLIENT.0),
            bottom: px(CLIENT.1),
        };
        let _ = AdjustWindowRectExForDpi(&mut rect, STYLE, false, EX_STYLE, dpi);
        let (width, height) = (rect.right - rect.left, rect.bottom - rect.top);
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let _ = GetMonitorInfoW(primary(), &mut info);
        let work = info.rcWork;
        let _ = SetWindowPos(
            hwnd,
            None,
            work.right - width - px(MARGIN),
            work.bottom - height - px(MARGIN),
            width,
            height,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            // Clicks land on the buttons without activating the window, so focus stays put.
            WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
            WM_CTLCOLORSTATIC => {
                SetBkColor(HDC(wparam.0 as _), COLORREF(GetSysColor(COLOR_WINDOW)));
                LRESULT(GetSysColorBrush(COLOR_WINDOW).0 as isize)
            }
            WM_COMMAND => {
                let (id, code) = ((wparam.0 & 0xFFFF) as i32, (wparam.0 >> 16) as u32);
                let press = match id {
                    ID_SKIP => Some(WM_POPUP_SKIP),
                    ID_SNOOZE => Some(WM_POPUP_SNOOZE),
                    _ => None,
                };
                if let Some(press) = press
                    && code == BN_CLICKED
                {
                    let owner = HWND(GetWindowLongPtrW(hwnd, GWLP_USERDATA) as _);
                    let _ = PostMessageW(Some(owner), press, WPARAM(0), LPARAM(0));
                }
                LRESULT(0)
            }
            // X hides the popup; the occurrence stays pending (SPEC §6).
            WM_CLOSE => {
                let _ = ShowWindow(hwnd, SW_HIDE);
                LRESULT(0)
            }
            WM_DPICHANGED => {
                layout(hwnd, (wparam.0 & 0xFFFF) as u32);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}
