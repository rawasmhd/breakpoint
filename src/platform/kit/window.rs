//! The kit window: a top-level window with the system caption and frame, its client area
//! painted from a [`Form`], and the drop-down menu as an owned popup window.
//!
//! - Frame: `WS_CAPTION | WS_SYSMENU`, the app icon, no resize. DWM tints the caption to the
//!   theme (`DWMWA_USE_IMMERSIVE_DARK_MODE`; on Windows 11 the caption, text and border colours).
//! - Theme: [`theme::app_theme`], read again on `WM_SETTINGCHANGE`; high contrast paints with the
//!   system colours ([`Colours::now`]) and no menu shadow.
//! - DPI: PerMonitorV2. `WM_DPICHANGED` lays the form out again and resizes the window to fit.
//! - Size: a window too tall for its monitor's work area takes the work area's height; the
//!   content above the footer band scrolls under the caption and the band stays in view
//!   ([`Scroll`], issue #84). The wheel, a painted scrollbar and keyboard focus scroll it. The
//!   window fits itself again on a DPI change, a move to another monitor and a work-area change.
//! - Input: keys and the pointer go to the [`Form`]; its [`Event`]s go to the caller's callback,
//!   called with no state borrowed, so it may call back through the [`Handle`], show a message
//!   box or close the window.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::sync::Once;
use std::time::Instant;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DWMWA_BORDER_COLOR, DWMWA_CAPTION_COLOR, DWMWA_COLOR_DEFAULT, DWMWA_TEXT_COLOR,
    DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
    DwmSetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, ClientToScreen, EndPaint, GetDC, GetMonitorInfoW, HMONITOR, InvalidateRect,
    MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromRect, MonitorFromWindow, PAINTSTRUCT,
    ReleaseDC, ScreenToClient,
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
    CS_DROPSHADOW, CreateWindowExW, DefWindowProcW, DestroyWindow, GWLP_USERDATA, GetClientRect,
    GetCursorPos, GetSystemMetrics, GetWindowLongPtrW, GetWindowRect, HICON, IDC_ARROW, IMAGE_ICON,
    IsWindow, KillTimer, LR_DEFAULTCOLOR, LoadCursorW, LoadImageW, MA_NOACTIVATE, RegisterClassExW,
    SM_CXICON, SM_CXSMICON, SM_CYICON, SM_CYSMICON, SPI_GETWHEELSCROLLLINES, SPI_SETWORKAREA,
    SW_SHOW, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOZORDER, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
    SetForegroundWindow, SetTimer, SetWindowLongPtrW, SetWindowPos, ShowWindow,
    SystemParametersInfoW, WA_INACTIVE, WINDOW_EX_STYLE, WINDOW_STYLE, WM_ACTIVATE,
    WM_CAPTURECHANGED, WM_CHAR, WM_CLOSE, WM_DISPLAYCHANGE, WM_DPICHANGED, WM_ENTERSIZEMOVE,
    WM_ERASEBKGND, WM_EXITSIZEMOVE, WM_KEYDOWN, WM_KILLFOCUS, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_MOUSEACTIVATE, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_MOVE, WM_NCDESTROY, WM_NCLBUTTONDOWN,
    WM_NCRBUTTONDOWN, WM_PAINT, WM_SETFOCUS, WM_SETTINGCHANGE, WM_SYSCOLORCHANGE, WM_SYSKEYDOWN,
    WM_THEMECHANGED, WM_TIMER, WNDCLASSEXW, WS_CAPTION, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_POPUP, WS_SYSMENU,
};
use windows::core::{BOOL, HSTRING, PCWSTR, Result, w};

use super::draw::{self, GdiMeasure};
use super::form::{Form, Key, MenuLayout, Scroll, inside, menu_layout, menu_place, rect};
use super::{Event, Id, Item, Value};
use crate::platform::paint::{self, FRAME, HOVER, REST, step_fill};
use crate::platform::theme::{
    self, Colours, FontCache, Fonts, MENU_GAP, SCROLL_LINE, SCROLL_ROOM, Theme, px,
};

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
    /// Client width and height at 96 DPI. The height grows at run time when wrapped text needs
    /// more lines at the window's DPI (see [`Form::lay_out`]). A height of 0 takes the bottom
    /// of the lowest item, such as the footer band.
    pub width: i32,
    pub height: i32,
    pub items: Vec<Item>,
    /// The window's top-left corner on screen, moved into the work area of its monitor if the
    /// window would not fit there. `None` centres it on the primary monitor's work area.
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
    /// The caller's callback; `None` while it runs.
    on_event: Option<Callback>,
    /// Events waiting for the callback, in order.
    queue: VecDeque<Event>,
    /// How the form scrolls when the window is shorter than the form.
    scroll: Scroll,
    /// The scrollbar's width level (0 at rest, 1 hot), whether the pointer is over it, and a
    /// thumb drag: the client y and the offset where it began.
    bar_level: f32,
    bar_hot: bool,
    drag: Option<(i32, i32)>,
    /// Whether the user is moving the window (between `WM_ENTERSIZEMOVE` and
    /// `WM_EXITSIZEMOVE`): then its place is the user's.
    sizing: bool,
}

impl Host {
    /// Lays the form out at `dpi` for a window on the work area `work`; returns the client
    /// size in pixels. The height is the form's, grown for wrapped text that needs more lines
    /// at this DPI, or as much as the work area leaves under the caption when that is less;
    /// then the form scrolls.
    fn relayout(&mut self, dpi: u32, work: RECT) -> (i32, i32) {
        let before = self.form.dpi();
        let fonts = self.fonts.at(dpi);
        with_screen_measure(fonts, |m| self.form.lay_out(dpi, m));
        let (w, full) = (px(self.size.0, dpi), self.form.height(self.size.1));
        let frame = window_size((w, full), dpi).1 - full;
        // The same place in the form across a DPI change.
        let offset = self.scroll.offset() * dpi as i32 / before.max(1) as i32;
        self.scroll = Scroll::new(full, self.form.footer(), work.bottom - work.top - frame);
        self.scroll.scroll_to(offset);
        self.client = (w, self.scroll.view());
        self.client
    }

    /// Scrolls the focused control into view with `scroll-room` round it.
    fn reveal_focus(&mut self) -> bool {
        let room = px(SCROLL_ROOM, self.form.dpi());
        let focused = self.form.focus_rect();
        focused.is_some_and(|r| self.scroll.reveal(r, room))
    }

    /// The pointer is at client (`x`, `y`): a thumb drag, the scrollbar's hover and the form's
    /// hover follow it. True when the look changed.
    fn hover(&mut self, x: i32, y: i32, down: bool) -> bool {
        let dpi = self.form.dpi();
        if let Some((from_y, from)) = self.drag {
            return self.scroll.drag(from, y - from_y, dpi);
        }
        let hot = self
            .scroll
            .bar(self.client.0, dpi)
            .is_some_and(|bar| inside(&bar, x, y));
        let bar = std::mem::replace(&mut self.bar_hot, hot) != hot;
        let (fx, fy) = self.scroll.in_form(x, y);
        self.form.pointer_move(fx, fy, down) || bar
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

/// Hands `events` to the caller's callback, in order, with no state borrowed.
///
/// Events raised while the callback runs (it pressed a key through `SendMessage`, focused a
/// stepper with typed text, or a message box let input through) wait in the queue, and the
/// running call hands them on after it returns, so none is lost. Stops if the callback closes
/// the window.
fn emit(hwnd: HWND, events: Vec<Event>) {
    if events.is_empty() {
        return;
    }
    let Some(mut callback) = with_host(hwnd, |h| {
        h.queue.extend(events);
        h.on_event.take()
    })
    .flatten() else {
        // The callback is running further up the stack; it takes these when it returns.
        return;
    };
    while let Some(event) = with_host(hwnd, |h| h.queue.pop_front()).flatten() {
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

    /// The item that has the focus inside the form.
    pub fn focused(self) -> Option<Id> {
        with_host(self.0, |h| h.form.focused()).flatten()
    }

    /// Where item `id` is on screen now, the scroll applied, as [`on_screen`] gives it.
    pub fn screen_rect(self, id: Id) -> Option<RECT> {
        with_host(self.0, |h| {
            let i = h.form.index(id)?;
            Some(on_screen(self.0, &h.scroll, h.form.laid()[i].rect))
        })
        .flatten()
    }

    /// Sets a control's value without an event, as when a profile fills the numbers.
    pub fn set(self, id: Id, value: Value) -> bool {
        let done = with_host(self.0, |h| h.form.set(id, value)).unwrap_or(false);
        refresh(self.0);
        done
    }

    /// Changes a text, label or drop-down `other` text, lays the form out again and resizes
    /// the window to fit, as [`Handle::fit`] does.
    pub fn set_text(self, id: Id, text: &str) -> bool {
        let done = with_host(self.0, |h| h.form.set_text(id, text)).unwrap_or(false);
        self.fit();
        done
    }

    /// Lays the form out again and resizes the window to its content, keeping its top-left
    /// corner (moved only as far as needed to stay in the work area). Call it after changes
    /// that alter text, such as a new work-week line in the wizard.
    pub fn fit(self) {
        let work = work_area(self.0);
        let Some((client, dpi)) = with_host(self.0, |h| {
            let dpi = h.form.dpi();
            (h.relayout(dpi, work), dpi)
        }) else {
            return;
        };
        let (w, h) = window_size(client, dpi);
        let now = self.window_rect();
        let (x, y) = keep_inside(now.left, now.top, w, h, work);
        // SAFETY: no state borrow is held.
        unsafe { move_to(self.0, rect(x, y, w, h)) };
        refresh(self.0);
    }

    /// Gives the window the keyboard focus, moves it to item `id`, scrolls it into view and
    /// shows the focus ring, as after a refused Save: the ring shows even when Save was
    /// clicked, so the user sees which control to fix.
    pub fn focus(self, id: Id) {
        let (events, scrolled) = with_host(self.0, |h| {
            let events = h.form.focus(id);
            (events, h.reveal_focus())
        })
        .unwrap_or_default();
        if scrolled {
            repoint(self.0);
        }
        // SAFETY: no pointers.
        let _ = unsafe { SetFocus(Some(self.0)) };
        after(self.0);
        emit(self.0, events);
    }
}

/// The open menu's window, for the shots.
#[cfg(test)]
pub(crate) fn menu_of(handle: Handle) -> Option<HWND> {
    with_host(handle.0, |h| h.menu).flatten()
}

/// The window's scroll, for tests.
#[cfg(test)]
pub(crate) fn scroll_of(handle: Handle) -> Option<Scroll> {
    with_host(handle.0, |h| h.scroll)
}

#[cfg(test)]
thread_local! {
    static PRETEND_WORK: std::cell::Cell<Option<RECT>> = const { std::cell::Cell::new(None) };
}

/// Makes kit windows opened or fitted on this thread take `work` as their monitor's work
/// area, so a test can try a small screen on any screen. `None` goes back to the real one.
#[cfg(test)]
pub(crate) fn pretend_work_area(work: Option<RECT>) {
    PRETEND_WORK.set(work);
}

/// The on-screen rectangle of `laid`, a rectangle in form pixels from the window's layout
/// (an item's [`super::form::Laid::rect`] or one of its parts), with the window's `scroll`
/// applied: where it shows now. Part of it may be scrolled out of view; [`Scroll::shown`]
/// says whether any of it shows. The drop-down menu opens from it, and screen readers (#80)
/// take control bounds from it. Sends no message, so it may run with the state borrowed.
///
/// The rectangle is not clipped: for a control partly under the footer band or past the top
/// of the client area, the caller must clip it to the part that shows.
pub fn on_screen(hwnd: HWND, scroll: &Scroll, laid: RECT) -> RECT {
    let r = scroll.on_client(laid);
    let mut corner = POINT::default();
    // SAFETY: `corner` is a valid in-out pointer; this sends no message.
    let _ = unsafe { ClientToScreen(hwnd, &mut corner) };
    RECT {
        left: r.left + corner.x,
        top: r.top + corner.y,
        right: r.right + corner.x,
        bottom: r.bottom + corner.y,
    }
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
            queue: VecDeque::new(),
            scroll: Scroll::default(),
            bar_level: REST,
            bar_hot: false,
            drag: None,
            sizing: false,
        }));
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(host) as isize);
        let dpi = GetDpiForWindow(hwnd);
        let work = work_area(hwnd);
        let client = with_host(hwnd, |h| h.relayout(dpi, work)).unwrap_or_default();
        let (w, h) = window_size(client, dpi);
        let (x, y) = match spec.at {
            Some(p) => keep_inside(p.x, p.y, w, h, work),
            None => (
                work.left + (work.right - work.left - w) / 2,
                work.top + (work.bottom - work.top - h) / 2,
            ),
        };
        let _ = SetWindowPos(hwnd, None, x, y, w, h, SWP_NOZORDER | SWP_NOACTIVATE);
        apply_frame(hwnd);
        Ok(Handle(hwnd))
    }
}

/// The top-left corner for a `w` by `h` window asked at (`x`, `y`), moved as little as needed
/// to lie inside `work`. A window larger than `work` keeps its top-left corner in it.
fn keep_inside(x: i32, y: i32, w: i32, h: i32, work: RECT) -> (i32, i32) {
    (
        x.min(work.right - w).max(work.left),
        y.min(work.bottom - h).max(work.top),
    )
}

/// The window size for a client area of `client` pixels at `dpi`.
pub(crate) fn window_size(client: (i32, i32), dpi: u32) -> (i32, i32) {
    let mut r = rect(0, 0, client.0, client.1);
    // SAFETY: `r` is a valid in-out pointer.
    let _ = unsafe { AdjustWindowRectExForDpi(&mut r, STYLE, false, WINDOW_EX_STYLE(0), dpi) };
    (r.right - r.left, r.bottom - r.top)
}

/// The work area of the monitor the window is on.
fn work_area(hwnd: HWND) -> RECT {
    // SAFETY: no pointers.
    work_of(unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) })
}

fn work_of(monitor: HMONITOR) -> RECT {
    #[cfg(test)]
    if let Some(work) = PRETEND_WORK.get() {
        return work;
    }
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: `info` is a valid MONITORINFO with its size set.
    let _ = unsafe { GetMonitorInfoW(monitor, &mut info) };
    info.rcWork
}

/// Fits the window again to the work area of the monitor it is on now: after a move to
/// another monitor, a change of the work area (the taskbar moved or grew) or of the display.
/// The window keeps its top-left corner as far as the work area allows. A window that fits
/// and keeps its size stays where it is; a window that scrolls is always moved inside the work
/// area, since it is as tall as the work area and Save is at its bottom.
fn refit(hwnd: HWND) {
    let work = work_area(hwnd);
    let Some((client, before, dpi, scrolls)) = with_host(hwnd, |h| {
        let (before, dpi) = (h.client, h.form.dpi());
        let client = h.relayout(dpi, work);
        (client, before, dpi, h.scroll.scrolls())
    }) else {
        return;
    };
    if client != before || scrolls {
        close_menu(hwnd);
        let (w, h) = window_size(client, dpi);
        let mut now = RECT::default();
        // SAFETY: a valid out-pointer.
        let _ = unsafe { GetWindowRect(hwnd, &mut now) };
        let (x, y) = keep_inside(now.left, now.top, w, h, work);
        // SAFETY: no state borrow is held.
        unsafe { move_to(hwnd, rect(x, y, w, h)) };
    }
    refresh(hwnd);
}

/// The lines one wheel notch scrolls, as the user set it; `u32::MAX` is a page.
fn wheel_lines() -> u32 {
    let mut lines = 3u32;
    // SAFETY: `lines` is a valid out-pointer of the size this action wants.
    let _ = unsafe {
        SystemParametersInfoW(
            SPI_GETWHEELSCROLLLINES,
            0,
            Some(&mut lines as *mut u32 as _),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    lines
}

/// After a scroll: the hover follows what is now under the pointer, if it is over the window.
fn repoint(hwnd: HWND) {
    let mut at = POINT::default();
    let mut client = RECT::default();
    // SAFETY: valid out-pointers; these send no message.
    unsafe {
        if GetCursorPos(&mut at).is_err() || !ScreenToClient(hwnd, &mut at).as_bool() {
            return;
        }
        let _ = GetClientRect(hwnd, &mut client);
    }
    if inside(&client, at.x, at.y) {
        // SAFETY: no pointers.
        let down = unsafe { GetKeyState(VK_LBUTTON.0.into()) } < 0;
        with_host(hwnd, |h| h.hover(at.x, at.y, down));
    }
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
        let form = h.form.tick(dt, animate);
        let bar = if h.bar_hot || h.drag.is_some() {
            HOVER
        } else {
            REST
        };
        h.bar_level = step_fill(h.bar_level, bar, dt, animate);
        form || h.bar_level != bar
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
            let scroll = h.scroll;
            let thumb = scroll.thumb(w, dpi, h.bar_level);
            let form = &h.form;
            let fonts = h.fonts.at(dpi);
            if scroll.scrolls() {
                draw::buffered_scrolled(
                    dc,
                    w,
                    &scroll,
                    &c,
                    fonts,
                    dpi,
                    thumb,
                    |canvas, m, runs| {
                        draw::paint_form(canvas, form, &looks, &c, m, runs);
                    },
                );
            } else {
                draw::buffered(dc, w, ht, c.ground, fonts, dpi, |canvas, m, runs| {
                    draw::paint_form(canvas, form, &looks, &c, m, runs);
                });
            }
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
            let laid = h.form.laid()[menu.item].rect;
            let client_w = h.client.0;
            let entries = h
                .form
                .menu_entries()
                .map(|(e, _)| e.to_vec())
                .unwrap_or_default();
            let fonts = h.fonts.at(dpi);
            let layout = with_screen_measure(fonts, |m| {
                menu_layout(&entries, laid.right - laid.left, dpi, m)
            });
            // Where the field shows now, after any scroll.
            let field = on_screen(hwnd, &h.scroll, laid);
            // The client area's right edge on screen.
            let edge = field.left - laid.left + client_w;
            let place = menu_place(
                field,
                layout.w,
                layout.h,
                px(MENU_GAP, dpi),
                edge,
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

/// Feeds `key` to the form and hands on what it returns. A key that moves the focus or acts
/// on the focused control scrolls that control into view; Enter and Esc, which press a
/// button, scroll nothing unless they move the focus.
fn on_key(hwnd: HWND, key: Key) {
    let (events, scrolled) = with_host(hwnd, |h| {
        let before = h.form.focused();
        let events = h.form.key(key);
        let scrolled = (!matches!(key, Key::Enter | Key::Esc) || h.form.focused() != before)
            && h.reveal_focus();
        (events, scrolled)
    })
    .unwrap_or_default();
    if scrolled {
        repoint(hwnd);
    }
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
                    let changed = h.hover(x, y, down);
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
                    h.bar_hot = false;
                    h.form.pointer_leave(down);
                });
                refresh(hwnd);
                LRESULT(0)
            }
            WM_LBUTTONDOWN => {
                let (x, y) = point(lparam);
                let on_bar = with_host(hwnd, |h| {
                    let bar = h.scroll.bar(h.client.0, h.form.dpi());
                    bar.is_some_and(|bar| inside(&bar, x, y))
                });
                if on_bar == Some(true) {
                    // The thumb drags; the track off the thumb pages toward the click.
                    close_menu(hwnd);
                    let paged = with_host(hwnd, |h| {
                        let dpi = h.form.dpi();
                        if h.scroll.on_thumb(y, dpi) {
                            h.drag = Some((y, h.scroll.offset()));
                            false
                        } else {
                            h.scroll.page_toward(y, dpi)
                        }
                    });
                    SetCapture(hwnd);
                    if paged == Some(true) {
                        repoint(hwnd);
                    }
                    refresh(hwnd);
                    return LRESULT(0);
                }
                let events = with_host(hwnd, |h| {
                    let (fx, fy) = h.scroll.in_form(x, y);
                    h.form.pointer_down(fx, fy)
                })
                .unwrap_or_default();
                SetCapture(hwnd);
                after(hwnd);
                emit(hwnd, events);
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                let (x, y) = point(lparam);
                if with_host(hwnd, |h| h.drag.take().is_some()) == Some(true) {
                    let _ = ReleaseCapture();
                    // The pointer may have left the bar during the drag.
                    repoint(hwnd);
                    refresh(hwnd);
                    return LRESULT(0);
                }
                let events = with_host(hwnd, |h| {
                    let (fx, fy) = h.scroll.in_form(x, y);
                    h.form.pointer_up(fx, fy)
                })
                .unwrap_or_default();
                let _ = ReleaseCapture();
                after(hwnd);
                emit(hwnd, events);
                LRESULT(0)
            }
            WM_CAPTURECHANGED => {
                with_host(hwnd, |h| {
                    h.drag = None;
                    h.form.pointer_cancel();
                });
                refresh(hwnd);
                LRESULT(0)
            }
            WM_MOUSEWHEEL => {
                let delta = i32::from((wparam.0 >> 16) as u16 as i16);
                let lines = wheel_lines();
                let moved = with_host(hwnd, |h| {
                    let line = px(SCROLL_LINE, h.form.dpi());
                    h.scroll.wheel(delta, lines, line)
                });
                if moved == Some(true) {
                    // The menu hangs from its field, which has moved.
                    close_menu(hwnd);
                    repoint(hwnd);
                    refresh(hwnd);
                }
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
                if msg == WM_SETTINGCHANGE && wparam.0 as u32 == SPI_SETWORKAREA.0 {
                    refit(hwnd);
                }
                refresh(hwnd);
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            // A move may end on a monitor of another size at the same DPI.
            WM_ENTERSIZEMOVE => {
                with_host(hwnd, |h| h.sizing = true);
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            WM_EXITSIZEMOVE | WM_DISPLAYCHANGE => {
                with_host(hwnd, |h| h.sizing = false);
                refit(hwnd);
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            WM_DPICHANGED => {
                let dpi = (wparam.0 & 0xffff) as u32;
                let suggested = *(lparam.0 as *const RECT);
                close_menu(hwnd);
                // The new monitor's work area. Windows' place for the window is kept while the
                // user drags it (the drag's end fits it again); otherwise a window that scrolls,
                // as tall as the work area, moves inside it, so Save is not under the taskbar.
                let work = work_of(MonitorFromRect(&suggested, MONITOR_DEFAULTTONEAREST));
                let fitted = with_host(hwnd, |h| {
                    (h.relayout(dpi, work), h.scroll.scrolls() && !h.sizing)
                });
                if let Some((client, inside_work)) = fitted {
                    let (w, h) = window_size(client, dpi);
                    let (x, y) = if inside_work {
                        keep_inside(suggested.left, suggested.top, w, h, work)
                    } else {
                        (suggested.left, suggested.top)
                    };
                    move_to(hwnd, rect(x, y, w, h));
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
    fn a_window_asked_off_its_work_area_is_moved_in() {
        let work = rect(0, 0, 1000, 800);
        assert_eq!(keep_inside(100, 100, 600, 400, work), (100, 100));
        assert_eq!(keep_inside(900, 700, 600, 400, work), (400, 400));
        assert_eq!(keep_inside(-50, -20, 600, 400, work), (0, 0));
        assert_eq!(keep_inside(300, 300, 1200, 900, work), (0, 0));
        let second = rect(1000, 0, 1920, 1040);
        assert_eq!(keep_inside(2800, 100, 600, 400, second), (2320, 100));
    }

    /// The callback presses a key through `SendMessage`; the toggle that raises arrives after
    /// the first call returns, not dropped.
    #[test]
    fn events_raised_while_the_callback_runs_are_handed_on_after_it() {
        use super::super::{Item, Kind, Place, Value};
        use std::rc::Rc;
        use windows::Win32::UI::WindowsAndMessaging::SendMessageW;

        let seen: Rc<RefCell<Vec<Event>>> = Rc::default();
        let log = seen.clone();
        let spec = Spec {
            caption: "kit test".into(),
            width: 100,
            height: 50,
            items: vec![Item::new(7, Place::at(8, 8), Kind::Switch { on: false })],
            at: Some(POINT { x: 0, y: 0 }),
            owner: None,
        };
        let space = |hwnd: HWND| unsafe {
            SendMessageW(
                hwnd,
                WM_KEYDOWN,
                Some(WPARAM(VK_SPACE.0.into())),
                Some(LPARAM(0)),
            );
        };
        let handle = open(spec, move |handle, event| {
            log.borrow_mut().push(event);
            if log.borrow().len() == 1 {
                space(handle.hwnd());
                // Still running: the second toggle waits for this call to return.
                assert_eq!(log.borrow().len(), 1);
            }
        })
        .unwrap();
        space(handle.hwnd());
        assert_eq!(
            *seen.borrow(),
            [
                Event::Changed(7, Value::On(true)),
                Event::Changed(7, Value::On(false))
            ]
        );
        assert_eq!(handle.value(7), Some(Value::On(false)));
        handle.close();
        assert!(!handle.is_open());
    }

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

    // ---- Scrolling (issue #84) ----

    use super::super::{ButtonKind, Entry, Kind, Place};
    use windows::Win32::UI::WindowsAndMessaging::{SWP_NOSIZE, SendMessageW};

    const SAVE: Id = 1;
    const DROP: Id = 2;

    /// A form 984 tall at 96 DPI: twenty switches, a drop-down near the bottom, and Save in a
    /// footer band.
    fn tall_spec() -> Spec {
        let mut items: Vec<Item> = (0..20)
            .map(|k| {
                Item::new(
                    100 + k,
                    Place::at(24, 24 + 40 * i32::from(k)),
                    Kind::Switch { on: false },
                )
            })
            .collect();
        items.push(Item::new(
            DROP,
            Place::new(24, 840, 160, 0),
            Kind::DropDown {
                entries: vec![Entry::new("One", None), Entry::new("Two", None)],
                current: Some(0),
                other: "Custom".into(),
            },
        ));
        items.push(Item::new(0, Place::new(0, 920, 300, 64), Kind::Band));
        items.push(Item::new(
            SAVE,
            Place::new(188, 936, 88, 0),
            Kind::Button {
                label: "Save".into(),
                kind: ButtonKind::Primary,
                default: true,
                cancel: true,
            },
        ));
        Spec {
            caption: "kit scroll test".into(),
            width: 300,
            height: 0,
            items,
            at: None,
            owner: None,
        }
    }

    /// A work area 400 tall at the top-left of the real one: too small for the tall form at
    /// any scale, on any screen.
    fn small_work() -> RECT {
        let real = work_of(unsafe { MonitorFromWindow(HWND::default(), MONITOR_DEFAULTTONEAREST) });
        rect(real.left, real.top, real.right - real.left, 400)
    }

    /// The window's client area on screen.
    fn client_on_screen(handle: Handle) -> RECT {
        let mut r = RECT::default();
        unsafe { GetClientRect(handle.hwnd(), &mut r).unwrap() };
        let mut corner = POINT::default();
        let _ = unsafe { ClientToScreen(handle.hwnd(), &mut corner) };
        rect(corner.x, corner.y, r.right, r.bottom)
    }

    fn within(inner: RECT, outer: RECT) -> bool {
        inner.left >= outer.left
            && inner.right <= outer.right
            && inner.top >= outer.top
            && inner.bottom <= outer.bottom
    }

    fn send(handle: Handle, msg: u32, wparam: usize) {
        unsafe {
            SendMessageW(handle.hwnd(), msg, Some(WPARAM(wparam)), Some(LPARAM(0)));
        }
    }

    /// One wheel notch toward the user (down) or away (up).
    fn wheel(handle: Handle, down: bool) {
        let delta: i16 = if down { -120 } else { 120 };
        send(handle, WM_MOUSEWHEEL, usize::from(delta as u16) << 16);
    }

    #[test]
    fn a_window_too_tall_for_its_work_area_scrolls_and_keeps_save_on_screen() {
        let work = small_work();
        pretend_work_area(Some(work));
        let handle = open(tall_spec(), |_, _| {}).unwrap();
        pretend_work_area(None);
        let window = handle.window_rect();
        assert!(within(window, work), "{window:?} in {work:?}");
        assert_eq!(
            window.bottom - window.top,
            400,
            "it takes the work area's height"
        );
        let scroll = scroll_of(handle).unwrap();
        assert!(scroll.scrolls());
        let client = client_on_screen(handle);
        assert_eq!(client.bottom - client.top, scroll.view());
        let save = handle.screen_rect(SAVE).unwrap();
        assert!(within(save, client), "Save {save:?} shows in {client:?}");

        // The wheel reaches the end, and Save stays where it was.
        let lines = wheel_lines();
        let mut notches = 0;
        while scroll_of(handle).unwrap().offset() < scroll.range() && notches < 200 {
            wheel(handle, true);
            notches += 1;
        }
        if lines > 0 {
            assert_eq!(scroll_of(handle).unwrap().offset(), scroll.range());
        }
        assert_eq!(handle.screen_rect(SAVE), Some(save));
        let last = handle.screen_rect(119).unwrap();
        if lines > 0 {
            assert!(within(last, client), "the last switch shows: {last:?}");
        }
        wheel(handle, false);
        if lines > 0 {
            assert!(scroll_of(handle).unwrap().offset() < scroll.range());
        }

        // Tab reaches every control, each scrolled fully into view, and wraps to the top.
        for _ in 0..22 {
            send(handle, WM_KEYDOWN, VK_TAB.0.into());
            let id = handle.focused().unwrap();
            let r = handle.screen_rect(id).unwrap();
            assert!(within(r, client), "{id} at {r:?} in {client:?}");
        }
        assert_eq!(handle.focused(), Some(100));
        // Back at the top, with `scroll-room` above the first switch.
        let dpi = unsafe { GetDpiForWindow(handle.hwnd()) };
        assert_eq!(
            scroll_of(handle).unwrap().offset(),
            (px(24, dpi) - px(SCROLL_ROOM, dpi)).max(0)
        );
        handle.close();
    }

    #[test]
    fn the_menu_opens_from_where_its_field_is_after_scrolling() {
        pretend_work_area(Some(small_work()));
        let handle = open(tall_spec(), |_, _| {}).unwrap();
        pretend_work_area(None);
        // Focus scrolls the drop-down into view; Space opens its menu there.
        handle.focus(DROP);
        assert!(scroll_of(handle).unwrap().offset() > 0);
        let field = handle.screen_rect(DROP).unwrap();
        assert!(within(field, client_on_screen(handle)));
        send(handle, WM_KEYDOWN, VK_SPACE.0.into());
        let menu = menu_of(handle).expect("the menu is open");
        let mut m = RECT::default();
        unsafe { GetWindowRect(menu, &mut m).unwrap() };
        let gap = px(MENU_GAP, unsafe { GetDpiForWindow(handle.hwnd()) });
        assert_eq!(m.left, field.left);
        assert!(
            m.top == field.bottom + gap || m.bottom == field.top - gap,
            "{m:?} hangs from {field:?}"
        );
        // The wheel moves the field away from its menu, so the menu closes.
        wheel(handle, false);
        if wheel_lines() > 0 {
            assert_eq!(menu_of(handle), None);
        }
        handle.close();
    }

    #[test]
    fn a_window_that_fits_its_work_area_does_not_scroll() {
        let mut spec = tall_spec();
        spec.items
            .retain(|item| item.id == SAVE || item.at.y < 200 || matches!(item.kind, Kind::Band));
        for item in &mut spec.items {
            if item.at.y >= 920 {
                item.at.y -= 720;
            }
        }
        let handle = open(spec, |_, _| {}).unwrap();
        let scroll = scroll_of(handle).unwrap();
        assert!(!scroll.scrolls());
        let client = client_on_screen(handle);
        assert_eq!(client.bottom - client.top, scroll.full());
        assert_eq!(
            scroll.full(),
            px(264, unsafe { GetDpiForWindow(handle.hwnd()) })
        );
        let offset = scroll.offset();
        wheel(handle, true);
        assert_eq!(scroll_of(handle).unwrap().offset(), offset);
        // Dragged partly past the bottom of the screen: it fits, so it stays where it was put.
        let work = work_area(handle.hwnd());
        drag_to(handle, work.left + 10, work.bottom - 50);
        let r = handle.window_rect();
        assert_eq!((r.left, r.top), (work.left + 10, work.bottom - 50));
        handle.close();
    }

    /// Moves the window's top-left to (`x`, `y`) as a user's drag would, then ends the move.
    fn drag_to(handle: Handle, x: i32, y: i32) {
        unsafe {
            SetWindowPos(
                handle.hwnd(),
                None,
                x,
                y,
                0,
                0,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            )
            .unwrap();
        }
        send(handle, WM_EXITSIZEMOVE, 0);
    }

    #[test]
    fn a_scrolling_window_stays_inside_its_work_area_after_a_scale_or_work_area_change() {
        let work = small_work();
        pretend_work_area(Some(work));
        let handle = open(tall_spec(), |_, _| {}).unwrap();
        let w = handle.window_rect().right - handle.window_rect().left;
        // The scale goes up by half. Windows suggests the old top-left, low on the work area;
        // the window, now as tall as the work area, moves up into it.
        let dpi = unsafe { GetDpiForWindow(handle.hwnd()) };
        let new = dpi * 3 / 2;
        let suggested = rect(work.left + 20, work.top + 89, w * 3 / 2, 600);
        unsafe {
            SendMessageW(
                handle.hwnd(),
                WM_DPICHANGED,
                Some(WPARAM((new | (new << 16)) as usize)),
                Some(LPARAM(&suggested as *const RECT as isize)),
            );
        }
        let r = handle.window_rect();
        assert!(within(r, work), "{r:?} in {work:?}");
        assert_eq!((r.left, r.bottom - r.top), (work.left + 20, 400));
        let save = handle.screen_rect(SAVE).unwrap();
        assert!(within(save, client_on_screen(handle)), "Save {save:?}");
        // The taskbar grows by 60: the window shrinks with the work area and stays in it.
        let less = rect(work.left, work.top, work.right - work.left, 340);
        pretend_work_area(Some(less));
        send(handle, WM_SETTINGCHANGE, SPI_SETWORKAREA.0 as usize);
        let r = handle.window_rect();
        assert!(within(r, less), "{r:?} in {less:?}");
        assert_eq!(r.bottom - r.top, 340);
        // Dragged down past the bottom: when the move ends, it comes back inside.
        drag_to(handle, less.left + 30, less.top + 100);
        let r = handle.window_rect();
        assert!(within(r, less), "{r:?} in {less:?}");
        assert_eq!(r.left, less.left + 30);
        pretend_work_area(None);
        handle.close();
    }
}
