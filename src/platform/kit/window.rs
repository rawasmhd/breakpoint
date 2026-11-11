//! The kit window: a top-level window with the system caption and frame, its client area
//! painted from a [`Form`], and the drop-down menu as an owned popup window.
//!
//! - Frame: `WS_CAPTION | WS_SYSMENU`, the app icon, no resize. DWM tints the caption to the
//!   theme (`DWMWA_USE_IMMERSIVE_DARK_MODE`; on Windows 11 the caption, text and border colours).
//! - Theme: [`theme::app_theme`], read again on `WM_SETTINGCHANGE`; high contrast paints with the
//!   system colours ([`Colours::now`]) and no menu shadow.
//! - DPI: PerMonitorV2. `WM_DPICHANGED` lays the form out again and resizes the window to fit.
//! - Input: keys and the pointer go to the [`Form`]; its [`Event`]s go to the caller's callback,
//!   called with no state borrowed, so it may call back through the [`Handle`], show a message
//!   box or close the window.

use std::cell::RefCell;
use std::sync::Once;
use std::time::Instant;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DWMWA_BORDER_COLOR, DWMWA_CAPTION_COLOR, DWMWA_COLOR_DEFAULT, DWMWA_TEXT_COLOR,
    DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
    DwmSetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, ClientToScreen, EndPaint, GetDC, GetMonitorInfoW, InvalidateRect,
    MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow, PAINTSTRUCT, ReleaseDC,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::SystemServices::MK_LBUTTON;
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForWindow};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, SetFocus, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent,
    VK_BACK, VK_DOWN, VK_END, VK_ESCAPE, VK_HOME, VK_LBUTTON, VK_LEFT, VK_NEXT, VK_PRIOR,
    VK_RETURN, VK_RIGHT, VK_SHIFT, VK_SPACE, VK_TAB, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CS_DROPSHADOW, CreateWindowExW, DefWindowProcW, DestroyWindow, GWLP_USERDATA, GetSystemMetrics,
    GetWindowLongPtrW, GetWindowRect, HICON, IDC_ARROW, IMAGE_ICON, IsWindow, KillTimer,
    LR_DEFAULTCOLOR, LoadCursorW, LoadImageW, MA_NOACTIVATE, RegisterClassExW, SM_CXICON,
    SM_CXSMICON, SM_CYICON, SM_CYSMICON, SW_SHOW, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOZORDER,
    SetForegroundWindow, SetTimer, SetWindowLongPtrW, SetWindowPos, ShowWindow, WA_INACTIVE,
    WINDOW_EX_STYLE, WINDOW_STYLE, WM_ACTIVATE, WM_CAPTURECHANGED, WM_CHAR, WM_CLOSE,
    WM_DPICHANGED, WM_ERASEBKGND, WM_KEYDOWN, WM_KILLFOCUS, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_MOUSEACTIVATE, WM_MOUSEMOVE, WM_MOVE, WM_NCDESTROY, WM_NCLBUTTONDOWN, WM_NCRBUTTONDOWN,
    WM_PAINT, WM_SETFOCUS, WM_SETTINGCHANGE, WM_SYSCOLORCHANGE, WM_SYSKEYDOWN, WM_THEMECHANGED,
    WM_TIMER, WNDCLASSEXW, WS_CAPTION, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_POPUP, WS_SYSMENU,
};
use windows::core::{BOOL, HSTRING, PCWSTR, Result, w};

use super::draw::{self, GdiMeasure};
use super::form::{Form, Key, MenuLayout, menu_layout, menu_place, rect};
use super::{Event, Id, Item, Value};
use crate::platform::paint::{self, FRAME};
use crate::platform::theme::{self, Colours, FontCache, Fonts, MENU_GAP, Theme, px};

/// The app icon in the caption and on the taskbar (`build.rs` embeds it as icon 1).
const APP_ICON: u16 = 1;
const CLASS: PCWSTR = w!("breakpoint.kit");
const MENU_CLASS: PCWSTR = w!("breakpoint.kit.menu");
/// The menu class without the drop shadow, for high contrast.
const MENU_FLAT_CLASS: PCWSTR = w!("breakpoint.kit.menu.flat");
const STYLE: WINDOW_STYLE = WINDOW_STYLE(WS_CAPTION.0 | WS_SYSMENU.0);
/// Runs the hover, press and knob timing, one repaint per [`FRAME`], while anything moves.
const FRAME_TIMER: usize = 1;

/// A form window to open: its caption, its client size at 96 DPI, its items, and where it goes.
pub struct Spec {
    pub caption: String,
    /// Client width and height at 96 DPI.
    pub width: i32,
    pub height: i32,
    pub items: Vec<Item>,
    /// The window's top-left corner on screen. `None` centres it on the primary monitor's work
    /// area.
    pub at: Option<POINT>,
    pub owner: Option<HWND>,
}

type Callback = Box<dyn FnMut(Handle, Event)>;

struct Host {
    form: Form,
    /// Client size at 96 DPI, then in pixels at the form's DPI.
    size: (i32, i32),
    client: (i32, i32),
    theme: Theme,
    high_contrast: bool,
    colours: Colours,
    fonts: FontCache,
    tracking: bool,
    last_frame: Instant,
    menu: Option<HWND>,
    menu_layout: Option<MenuLayout>,
    /// No DWM round corners (Windows 10): the menu paints its own outline.
    menu_outline: bool,
    on_event: Option<Callback>,
}

impl Host {
    /// Lays the form out at `dpi`; returns the client size in pixels.
    fn relayout(&mut self, dpi: u32) -> (i32, i32) {
        let fonts = self.fonts.at(dpi);
        with_screen_measure(fonts, |m| self.form.lay_out(dpi, m));
        self.client = (px(self.size.0, dpi), px(self.size.1, dpi));
        self.client
    }

    fn restyle(&mut self) {
        self.theme = theme::app_theme();
        self.high_contrast = theme::high_contrast();
        self.colours = Colours::now(self.theme);
    }
}

fn with_screen_measure<R>(fonts: &Fonts, f: impl FnOnce(&GdiMeasure) -> R) -> R {
    // SAFETY: the screen DC is released before return.
    unsafe {
        let dc = GetDC(None);
        let out = f(&GdiMeasure { dc, fonts });
        ReleaseDC(None, dc);
        out
    }
}

fn host(hwnd: HWND) -> Option<&'static RefCell<Host>> {
    // SAFETY: GWLP_USERDATA holds the Box made in `open`, alive until WM_NCDESTROY frees it.
    unsafe {
        let p = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const RefCell<Host>;
        (!p.is_null()).then(|| &*p)
    }
}

/// Runs `f` on the window's state, unless there is none or it is in use (a message sent while
/// another handler runs). Never call into Win32 from `f` in a way that sends this window a
/// message.
fn with_host<R>(hwnd: HWND, f: impl FnOnce(&mut Host) -> R) -> Option<R> {
    let cell = host(hwnd)?;
    let mut h = cell.try_borrow_mut().ok()?;
    Some(f(&mut h))
}

/// Hands `events` to the caller's callback with no state borrowed. Stops if the callback
/// closes the window.
fn emit(hwnd: HWND, events: Vec<Event>) {
    if events.is_empty() {
        return;
    }
    let Some(mut callback) = with_host(hwnd, |h| h.on_event.take()).flatten() else {
        return;
    };
    for event in events {
        callback(Handle(hwnd), event);
        // SAFETY: no pointers.
        if !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
            return;
        }
    }
    with_host(hwnd, |h| {
        h.on_event.get_or_insert(callback);
    });
}

/// A kit window. Copy it freely; every call checks the window is still there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Handle(HWND);

impl Handle {
    pub fn hwnd(self) -> HWND {
        self.0
    }

    pub fn is_open(self) -> bool {
        // SAFETY: no pointers.
        unsafe { IsWindow(Some(self.0)) }.as_bool()
    }

    /// Shows the window and brings it to the front.
    pub fn show(self) {
        // SAFETY: no pointers; a stale handle fails harmlessly.
        unsafe {
            let _ = ShowWindow(self.0, SW_SHOW);
            let _ = SetForegroundWindow(self.0);
        }
    }

    /// Destroys the window. Its callback is dropped.
    pub fn close(self) {
        // SAFETY: as above.
        let _ = unsafe { DestroyWindow(self.0) };
    }

    /// The window's rectangle on screen, frame included.
    pub fn window_rect(self) -> RECT {
        let mut r = RECT::default();
        // SAFETY: `r` is a valid out-pointer.
        let _ = unsafe { GetWindowRect(self.0, &mut r) };
        r
    }

    pub fn value(self, id: Id) -> Option<Value> {
        with_host(self.0, |h| h.form.value(id)).flatten()
    }

    /// Sets a control's value without an event, as when a profile fills the numbers.
    pub fn set(self, id: Id, value: Value) -> bool {
        let done = with_host(self.0, |h| h.form.set(id, value)).unwrap_or(false);
        refresh(self.0);
        done
    }

    /// Changes a text, label or drop-down `other` text, and lays the form out again.
    pub fn set_text(self, id: Id, text: &str) -> bool {
        let done = with_host(self.0, |h| {
            let done = h.form.set_text(id, text);
            let dpi = h.form.dpi();
            h.relayout(dpi);
            done
        })
        .unwrap_or(false);
        refresh(self.0);
        done
    }

    /// Gives the window the keyboard focus and moves it to item `id`, as after a refused Save.
    pub fn focus(self, id: Id) {
        let events = with_host(self.0, |h| h.form.focus(id)).unwrap_or_default();
        // SAFETY: no pointers.
        let _ = unsafe { SetFocus(Some(self.0)) };
        after(self.0);
        emit(self.0, events);
    }
}

/// The open menu's window, for the shots.
#[cfg(test)]
pub(super) fn menu_of(handle: Handle) -> Option<HWND> {
    with_host(handle.0, |h| h.menu).flatten()
}

fn register() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        // SAFETY: static class names; icons and cursor are loaded or left empty.
        unsafe {
            let Ok(module) = GetModuleHandleW(None) else {
                return;
            };
            let instance = module.into();
            let icon = |size: (_, _)| {
                LoadImageW(
                    Some(instance),
                    PCWSTR(usize::from(APP_ICON) as *const u16),
                    IMAGE_ICON,
                    GetSystemMetrics(size.0),
                    GetSystemMetrics(size.1),
                    LR_DEFAULTCOLOR,
                )
                .map(|h| HICON(h.0))
                .unwrap_or_default()
            };
            let cursor = LoadCursorW(None, IDC_ARROW).unwrap_or_default();
            RegisterClassExW(&WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(wndproc),
                hInstance: instance,
                hIcon: icon((SM_CXICON, SM_CYICON)),
                hIconSm: icon((SM_CXSMICON, SM_CYSMICON)),
                hCursor: cursor,
                lpszClassName: CLASS,
                ..Default::default()
            });
            for (class, style) in [
                (MENU_CLASS, CS_DROPSHADOW),
                (MENU_FLAT_CLASS, Default::default()),
            ] {
                RegisterClassExW(&WNDCLASSEXW {
                    cbSize: size_of::<WNDCLASSEXW>() as u32,
                    style,
                    lpfnWndProc: Some(menu_proc),
                    hInstance: instance,
                    hCursor: cursor,
                    lpszClassName: class,
                    ..Default::default()
                });
            }
        }
    });
}

/// Opens a kit window, hidden; call [`Handle::show`]. `on_event` gets what the user does.
pub fn open(spec: Spec, on_event: impl FnMut(Handle, Event) + 'static) -> Result<Handle> {
    paint::startup();
    register();
    let theme = theme::app_theme();
    let at = spec.at.unwrap_or_default();
    // SAFETY: plain Win32 window creation; the state box lives until WM_NCDESTROY.
    unsafe {
        let instance = GetModuleHandleW(None)?;
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            CLASS,
            &HSTRING::from(spec.caption.as_str()),
            STYLE,
            at.x,
            at.y,
            0,
            0,
            spec.owner,
            None,
            Some(instance.into()),
            None,
        )?;
        let host = Box::new(RefCell::new(Host {
            form: Form::new(spec.items),
            size: (spec.width, spec.height),
            client: (0, 0),
            theme,
            high_contrast: theme::high_contrast(),
            colours: Colours::now(theme),
            fonts: FontCache::new(),
            tracking: false,
            last_frame: Instant::now(),
            menu: None,
            menu_layout: None,
            menu_outline: false,
            on_event: Some(Box::new(on_event)),
        }));
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(host) as isize);
        let dpi = GetDpiForWindow(hwnd);
        let client = with_host(hwnd, |h| h.relayout(dpi)).unwrap_or_default();
        let (w, h) = window_size(client, dpi);
        let (x, y) = match spec.at {
            Some(p) => (p.x, p.y),
            None => {
                let work = work_area(hwnd);
                (
                    work.left + (work.right - work.left - w) / 2,
                    work.top + (work.bottom - work.top - h) / 2,
                )
            }
        };
        let _ = SetWindowPos(hwnd, None, x, y, w, h, SWP_NOZORDER | SWP_NOACTIVATE);
        apply_frame(hwnd);
        Ok(Handle(hwnd))
    }
}

/// The window size for a client area of `client` pixels at `dpi`.
fn window_size(client: (i32, i32), dpi: u32) -> (i32, i32) {
    let mut r = rect(0, 0, client.0, client.1);
    // SAFETY: `r` is a valid in-out pointer.
    let _ = unsafe { AdjustWindowRectExForDpi(&mut r, STYLE, false, WINDOW_EX_STYLE(0), dpi) };
    (r.right - r.left, r.bottom - r.top)
}

fn work_area(hwnd: HWND) -> RECT {
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: `info` is a valid MONITORINFO with its size set.
    let _ =
        unsafe { GetMonitorInfoW(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST), &mut info) };
    info.rcWork
}

/// Tints the frame to the theme: dark mode, then on Windows 11 the caption, its text and the
/// border. High contrast leaves the frame to Windows.
fn apply_frame(hwnd: HWND) {
    let Some((theme, hc, c)) = with_host(hwnd, |h| (h.theme, h.high_contrast, h.colours)) else {
        return;
    };
    let dark = BOOL::from(theme == Theme::Dark && !hc);
    // SAFETY: each value lives through its call and has the size given. Windows 10 does not
    // know the colour attributes and says no, which is fine.
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            &dark as *const BOOL as _,
            size_of::<BOOL>() as u32,
        );
        for (attr, colour) in [
            (DWMWA_CAPTION_COLOR, c.ground),
            (DWMWA_TEXT_COLOR, c.ink),
            (DWMWA_BORDER_COLOR, c.hairline),
        ] {
            let value = if hc { DWMWA_COLOR_DEFAULT } else { colour.0 };
            let _ = DwmSetWindowAttribute(
                hwnd,
                attr,
                &value as *const u32 as _,
                size_of::<u32>() as u32,
            );
        }
    }
}

// ---- Frames and repaint ----

/// Repaints and runs frames until the form settles.
fn refresh(hwnd: HWND) {
    with_host(hwnd, |h| h.last_frame = Instant::now());
    // SAFETY: no pointers; a stale handle fails harmlessly.
    unsafe {
        let _ = InvalidateRect(Some(hwnd), None, false);
        SetTimer(Some(hwnd), FRAME_TIMER, FRAME.as_millis() as u32, None);
    }
}

/// After any input: the menu window follows the form's menu, and the form repaints.
fn after(hwnd: HWND) {
    sync_menu(hwnd);
    refresh(hwnd);
}

fn frame(hwnd: HWND) {
    let animate = theme::animations_on();
    let moving = with_host(hwnd, |h| {
        let now = Instant::now();
        let dt = now - h.last_frame;
        h.last_frame = now;
        h.form.tick(dt, animate)
    });
    // SAFETY: no pointers.
    unsafe {
        let _ = InvalidateRect(Some(hwnd), None, false);
        if moving == Some(false) {
            let _ = KillTimer(Some(hwnd), FRAME_TIMER);
        }
    }
}

fn paint(hwnd: HWND) {
    let mut ps = PAINTSTRUCT::default();
    // SAFETY: Begin/EndPaint pair on the live window.
    unsafe {
        let dc = BeginPaint(hwnd, &mut ps);
        with_host(hwnd, |h| {
            let dpi = h.form.dpi();
            let (w, ht) = h.client;
            let c = h.colours;
            let looks = h.form.looks();
            let form = &h.form;
            let fonts = h.fonts.at(dpi);
            draw::buffered(dc, w, ht, c.ground, fonts, dpi, |canvas, m, runs| {
                draw::paint_form(canvas, form, &looks, &c, m, runs);
            });
        });
        let _ = EndPaint(hwnd, &ps);
    }
}

// ---- The menu window ----

enum MenuPlan {
    Show {
        place: RECT,
        existing: Option<HWND>,
        flat: bool,
    },
    Close(HWND),
    Nothing,
}

/// Opens, moves, repaints or closes the menu window to match the form's menu.
fn sync_menu(hwnd: HWND) {
    let plan = with_host(hwnd, |h| match (h.form.menu(), h.menu) {
        (Some(menu), existing) => {
            let dpi = h.form.dpi();
            let field = h.form.laid()[menu.item].rect;
            let entries = h
                .form
                .menu_entries()
                .map(|(e, _)| e.to_vec())
                .unwrap_or_default();
            let fonts = h.fonts.at(dpi);
            let layout = with_screen_measure(fonts, |m| {
                menu_layout(&entries, field.right - field.left, dpi, m)
            });
            let mut corner = POINT {
                x: field.left,
                y: field.top,
            };
            // SAFETY: `corner` is a valid in-out pointer; this sends no message.
            let _ = unsafe { ClientToScreen(hwnd, &mut corner) };
            let field = rect(
                corner.x,
                corner.y,
                field.right - field.left,
                field.bottom - field.top,
            );
            let place = menu_place(
                field,
                layout.w,
                layout.h,
                px(MENU_GAP, dpi),
                work_area(hwnd),
            );
            h.menu_layout = Some(layout);
            MenuPlan::Show {
                place,
                existing,
                flat: !h.colours.shadows,
            }
        }
        (None, Some(menu)) => {
            h.menu = None;
            h.menu_layout = None;
            MenuPlan::Close(menu)
        }
        (None, None) => MenuPlan::Nothing,
    });
    // SAFETY: no state borrow is held; every handle is one this window owns.
    unsafe {
        match plan {
            Some(MenuPlan::Show {
                place,
                existing: Some(menu),
                ..
            }) => {
                move_to(menu, place);
                let _ = InvalidateRect(Some(menu), None, false);
            }
            Some(MenuPlan::Show {
                place,
                existing: None,
                flat,
            }) => {
                let class = if flat { MENU_FLAT_CLASS } else { MENU_CLASS };
                let Ok(instance) = GetModuleHandleW(None) else {
                    return;
                };
                let Ok(menu) = CreateWindowExW(
                    WINDOW_EX_STYLE(WS_EX_TOOLWINDOW.0 | WS_EX_NOACTIVATE.0),
                    class,
                    None,
                    WS_POPUP,
                    place.left,
                    place.top,
                    place.right - place.left,
                    place.bottom - place.top,
                    Some(hwnd),
                    None,
                    Some(instance.into()),
                    None,
                ) else {
                    return;
                };
                SetWindowLongPtrW(menu, GWLP_USERDATA, hwnd.0 as isize);
                let round = DwmSetWindowAttribute(
                    menu,
                    DWMWA_WINDOW_CORNER_PREFERENCE,
                    &DWMWCP_ROUND as *const _ as _,
                    size_of_val(&DWMWCP_ROUND) as u32,
                );
                let border = with_host(hwnd, |h| {
                    h.menu = Some(menu);
                    h.menu_outline = round.is_err();
                    if h.high_contrast {
                        DWMWA_COLOR_DEFAULT
                    } else {
                        h.colours.hairline.0
                    }
                })
                .unwrap_or(DWMWA_COLOR_DEFAULT);
                let _ = DwmSetWindowAttribute(
                    menu,
                    DWMWA_BORDER_COLOR,
                    &border as *const u32 as _,
                    size_of::<u32>() as u32,
                );
                let _ = ShowWindow(menu, SW_SHOWNOACTIVATE);
            }
            Some(MenuPlan::Close(menu)) => {
                let _ = DestroyWindow(menu);
            }
            _ => {}
        }
    }
}

unsafe fn move_to(hwnd: HWND, place: RECT) {
    // SAFETY: per the caller, `hwnd` is live and no state borrow is held.
    let _ = unsafe {
        SetWindowPos(
            hwnd,
            None,
            place.left,
            place.top,
            place.right - place.left,
            place.bottom - place.top,
            SWP_NOZORDER | SWP_NOACTIVATE,
        )
    };
}

fn close_menu(hwnd: HWND) {
    if with_host(hwnd, |h| h.form.menu().is_some()) == Some(true) {
        with_host(hwnd, |h| h.form.close_menu());
        after(hwnd);
    }
}

fn paint_menu(menu: HWND, owner: HWND) {
    let mut ps = PAINTSTRUCT::default();
    // SAFETY: Begin/EndPaint pair on the live menu window.
    unsafe {
        let dc = BeginPaint(menu, &mut ps);
        with_host(owner, |h| {
            let (Some(layout), Some((entries, current)), Some(open)) =
                (h.menu_layout.as_ref(), h.form.menu_entries(), h.form.menu())
            else {
                return;
            };
            let dpi = h.form.dpi();
            let c = h.colours;
            let outline = h.menu_outline;
            let fonts = h.fonts.at(dpi);
            draw::buffered(
                dc,
                layout.w,
                layout.h,
                c.raised,
                fonts,
                dpi,
                |canvas, _, runs| {
                    draw::paint_menu(
                        canvas, layout, entries, current, open.hot, outline, &c, dpi, runs,
                    );
                },
            );
        });
        let _ = EndPaint(menu, &ps);
    }
}

unsafe extern "system" fn menu_proc(
    menu: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // SAFETY: GWLP_USERDATA holds the owner form window, set right after creation.
    let owner = HWND(unsafe { GetWindowLongPtrW(menu, GWLP_USERDATA) } as _);
    // SAFETY: Win32 calls on the live menu window; no state borrow is held across a call that
    // sends a message.
    unsafe {
        match msg {
            // Clicks land without activating, so the form keeps the keyboard focus.
            WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
            WM_ERASEBKGND => LRESULT(1),
            WM_PAINT => {
                paint_menu(menu, owner);
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                let (x, y) = point(lparam);
                let changed = with_host(owner, |h| {
                    let hit = h.menu_layout.as_ref().and_then(|l| l.hit(x, y));
                    h.form.menu_hover(hit)
                });
                if changed == Some(true) {
                    let _ = InvalidateRect(Some(menu), None, false);
                }
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                let (x, y) = point(lparam);
                let events = with_host(owner, |h| {
                    let hit = h.menu_layout.as_ref().and_then(|l| l.hit(x, y));
                    hit.map(|k| h.form.menu_pick(k)).unwrap_or_default()
                })
                .unwrap_or_default();
                after(owner);
                emit(owner, events);
                LRESULT(0)
            }
            _ => DefWindowProcW(menu, msg, wparam, lparam),
        }
    }
}

// ---- Window procedure ----

fn point(lparam: LPARAM) -> (i32, i32) {
    let x = (lparam.0 & 0xffff) as u16 as i16;
    let y = ((lparam.0 >> 16) & 0xffff) as u16 as i16;
    (x.into(), y.into())
}

/// The form key for a key-down message, or `None` for keys the form does not take.
fn key_for(vk: u16, shift: bool, alt: bool) -> Option<Key> {
    let key = match windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY(vk) {
        VK_TAB if shift => Key::BackTab,
        VK_TAB => Key::Tab,
        VK_RETURN => Key::Enter,
        VK_ESCAPE => Key::Esc,
        VK_SPACE => Key::Space,
        VK_UP if alt => Key::AltUp,
        VK_DOWN if alt => Key::AltDown,
        VK_UP => Key::Up,
        VK_DOWN => Key::Down,
        VK_LEFT => Key::Left,
        VK_RIGHT => Key::Right,
        VK_PRIOR => Key::PageUp,
        VK_NEXT => Key::PageDown,
        VK_HOME => Key::Home,
        VK_END => Key::End,
        VK_BACK => Key::Back,
        _ => return None,
    };
    // With Alt, only Alt+Up and Alt+Down; the rest stay Windows' (Alt+F4, Alt+Space).
    (!alt || matches!(key, Key::AltUp | Key::AltDown)).then_some(key)
}

/// Feeds `key` to the form and hands on what it returns.
fn on_key(hwnd: HWND, key: Key) {
    let events = with_host(hwnd, |h| h.form.key(key)).unwrap_or_default();
    after(hwnd);
    emit(hwnd, events);
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: Win32 calls on the live window; no state borrow is held across a call that sends
    // this window a message.
    unsafe {
        match msg {
            WM_ERASEBKGND => LRESULT(1),
            WM_PAINT => {
                paint(hwnd);
                LRESULT(0)
            }
            WM_SETFOCUS => {
                with_host(hwnd, |h| h.form.focus_in());
                refresh(hwnd);
                LRESULT(0)
            }
            WM_KILLFOCUS => {
                let events = with_host(hwnd, |h| h.form.focus_out()).unwrap_or_default();
                after(hwnd);
                emit(hwnd, events);
                LRESULT(0)
            }
            WM_ACTIVATE => {
                if (wparam.0 & 0xffff) as u32 == WA_INACTIVE {
                    close_menu(hwnd);
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            WM_KEYDOWN | WM_SYSKEYDOWN => {
                let shift = GetKeyState(VK_SHIFT.0.into()) < 0;
                let alt = msg == WM_SYSKEYDOWN;
                let repeat = lparam.0 & (1 << 30) != 0;
                match key_for(wparam.0 as u16, shift, alt) {
                    // A held key repeats steps and moves, not presses and toggles.
                    Some(Key::Space | Key::Enter | Key::Esc) if repeat => LRESULT(0),
                    Some(key) => {
                        on_key(hwnd, key);
                        LRESULT(0)
                    }
                    None => DefWindowProcW(hwnd, msg, wparam, lparam),
                }
            }
            WM_CHAR => {
                if let Some(c) = char::from_u32(wparam.0 as u32)
                    && (c.is_ascii_digit() || c == ':')
                {
                    on_key(hwnd, Key::Char(c));
                }
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                let (x, y) = point(lparam);
                let down = wparam.0 & MK_LBUTTON.0 as usize != 0;
                let moved = with_host(hwnd, |h| {
                    let changed = h.form.pointer_move(x, y, down);
                    let track = !std::mem::replace(&mut h.tracking, true);
                    (changed, track)
                });
                if let Some((changed, track)) = moved {
                    if track {
                        let mut event = TRACKMOUSEEVENT {
                            cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                            dwFlags: TME_LEAVE,
                            hwndTrack: hwnd,
                            dwHoverTime: 0,
                        };
                        let _ = TrackMouseEvent(&mut event);
                    }
                    if changed {
                        refresh(hwnd);
                    }
                }
                LRESULT(0)
            }
            WM_MOUSELEAVE => {
                let down = GetKeyState(VK_LBUTTON.0.into()) < 0;
                with_host(hwnd, |h| {
                    h.tracking = false;
                    h.form.pointer_leave(down);
                });
                refresh(hwnd);
                LRESULT(0)
            }
            WM_LBUTTONDOWN => {
                let (x, y) = point(lparam);
                let events = with_host(hwnd, |h| h.form.pointer_down(x, y)).unwrap_or_default();
                SetCapture(hwnd);
                after(hwnd);
                emit(hwnd, events);
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                let (x, y) = point(lparam);
                let events = with_host(hwnd, |h| h.form.pointer_up(x, y)).unwrap_or_default();
                let _ = ReleaseCapture();
                after(hwnd);
                emit(hwnd, events);
                LRESULT(0)
            }
            WM_CAPTURECHANGED => {
                with_host(hwnd, |h| h.form.pointer_cancel());
                refresh(hwnd);
                LRESULT(0)
            }
            WM_NCLBUTTONDOWN | WM_NCRBUTTONDOWN => {
                close_menu(hwnd);
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            WM_MOVE => {
                close_menu(hwnd);
                LRESULT(0)
            }
            WM_TIMER if wparam.0 == FRAME_TIMER => {
                frame(hwnd);
                LRESULT(0)
            }
            WM_SETTINGCHANGE | WM_SYSCOLORCHANGE | WM_THEMECHANGED => {
                with_host(hwnd, |h| h.restyle());
                apply_frame(hwnd);
                close_menu(hwnd);
                refresh(hwnd);
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            WM_DPICHANGED => {
                let dpi = (wparam.0 & 0xffff) as u32;
                let suggested = *(lparam.0 as *const RECT);
                close_menu(hwnd);
                if let Some(client) = with_host(hwnd, |h| h.relayout(dpi)) {
                    let (w, h) = window_size(client, dpi);
                    move_to(hwnd, rect(suggested.left, suggested.top, w, h));
                    refresh(hwnd);
                }
                LRESULT(0)
            }
            WM_CLOSE => {
                let events = with_host(hwnd, |h| vec![h.form.escape()]).unwrap_or_default();
                emit(hwnd, events);
                LRESULT(0)
            }
            WM_NCDESTROY => {
                let p = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut RefCell<Host>;
                if !p.is_null() {
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                    let host = Box::from_raw(p);
                    if let Some(menu) = host.borrow().menu {
                        let _ = DestroyWindow(menu);
                    }
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_map_to_form_keys() {
        assert_eq!(key_for(VK_TAB.0, false, false), Some(Key::Tab));
        assert_eq!(key_for(VK_TAB.0, true, false), Some(Key::BackTab));
        assert_eq!(key_for(VK_DOWN.0, false, true), Some(Key::AltDown));
        assert_eq!(key_for(VK_UP.0, false, true), Some(Key::AltUp));
        assert_eq!(
            key_for(VK_SPACE.0, false, true),
            None,
            "Alt+Space is the window menu"
        );
        assert_eq!(key_for(VK_NEXT.0, false, false), Some(Key::PageDown));
        assert_eq!(key_for(b'A'.into(), false, false), None);
    }
}
