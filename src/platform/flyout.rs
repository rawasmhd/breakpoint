//! The tray flyout (SPEC §8, issue #89): a painted panel next to the notification icon, in
//! place of a native menu, built on the form kit as a [`Frame::Popup`] window.
//!
//! - [`content`] maps the app's state (the SPEC §7 switch item, the pending prompt, the Today
//!   count) to what the flyout shows, and [`items`] lays that out at 96 DPI. Both are pure.
//! - [`place`] puts the flyout next to the icon and the taskbar edge, inside the work area. Pure
//!   too; [`open`] feeds it `Shell_NotifyIconGetRect` (from `tray-icon`) and
//!   `SHAppBarMessage(ABM_GETTASKBARPOS)`.
//! - The window takes focus and activates (unlike the popup), and closes on Esc, on losing
//!   activation (a click outside it, a second click on the icon) and after any action. What
//!   the user picks goes to the main loop as a [`Command`].
//! - If Windows refuses it the foreground (the foreground lock), it joins the foreground
//!   thread's input for a moment and asks again, as tray menus do. If that fails too, a timer
//!   closes it on the next click outside it, so it never stays up without the focus.
//! - The app hands it its content on every change and every second ([`show_content`]), so a
//!   click on the icon opens it from the icon's own event handler, also during a modal loop.

use std::cell::{Cell, RefCell};
use std::time::{Duration, Instant};

use breakpoint::core::quiet::SwitchItem;
use breakpoint::core::scheduler::{Nudge, Prompt, WATER_LINE, countdown_text};
use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromRect,
};
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, GetDoubleClickTime, VK_LBUTTON, VK_MBUTTON, VK_RBUTTON,
};
use windows::Win32::UI::Shell::{
    ABE_BOTTOM, ABE_LEFT, ABE_RIGHT, ABE_TOP, ABM_GETTASKBARPOS, APPBARDATA, SHAppBarMessage,
};
use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, GetCursorPos, GetForegroundWindow, GetWindowThreadProcessId, KillTimer,
    SetForegroundWindow, SetTimer,
};

use super::kit::window::{self, Frame, Handle, Spec};
use super::kit::{ButtonKind, Event, Id, Ink, Item, Kind, Place, Span, Value};
use super::paint::{Glyph, Mark};
use super::popup::split_digits;
use super::theme::{
    SIZE_BUTTON, SIZE_FLYOUT, SIZE_GLYPH, SIZE_GLYPH_INLINE, SPACE_1, SPACE_2, SPACE_3, SPACE_4,
    STATUS_DOT, SWITCH_H, SWITCH_W, Style, px,
};
use super::tray::{self, Command};

// ---- What it shows ----

/// The pending occurrence's card.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pending {
    pub nudge: Nudge,
    /// The away time still needed, for the live countdown.
    pub left: Duration,
    /// The riding Water line shows, as in the popup.
    pub water: bool,
    /// Snooze 5 min shows: not once the one snooze is used.
    pub snooze: bool,
}

/// Everything the flyout shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Content {
    /// The Nudges switch is on and the status dot filled: nudging, in hours or in Overtime.
    pub on: bool,
    /// The status line under "breakpoint", from SPEC §7: "On until 17:00".
    pub status: String,
    /// What flipping the switch does now.
    pub note: String,
    pub pending: Option<Pending>,
    /// The Today count (SPEC §10.3): taken and skipped.
    pub taken: u32,
    pub skipped: u32,
}

/// What the flyout shows for the switch item `item` (SPEC §7), the pending `prompt` with
/// `left` of away time still needed (SPEC §6), and the Today count `today` (taken, skipped).
pub fn content(
    item: &SwitchItem,
    prompt: Option<&Prompt>,
    left: Duration,
    today: (u32, u32),
) -> Content {
    Content {
        on: item.on(),
        status: item.status.clone(),
        note: item.note.clone(),
        pending: prompt.map(|p| Pending {
            nudge: p.nudge,
            left,
            water: p.water,
            snooze: p.snooze,
        }),
        taken: today.0,
        skipped: today.1,
    }
}

impl Content {
    /// Whether `other` differs from this only in the countdown, which then changes in place.
    fn same_but_countdown(&self, other: &Content) -> bool {
        let strip = |c: &Content| Content {
            pending: c.pending.clone().map(|p| Pending {
                left: Duration::ZERO,
                ..p
            }),
            ..c.clone()
        };
        strip(self) == strip(other)
    }
}

// ---- Layout, at 96 DPI ----

pub const SWITCH: Id = 1;
pub const SNOOZE: Id = 2;
pub const SKIP: Id = 3;
pub const SETTINGS: Id = 4;
pub const QUIT: Id = 5;
pub const COUNTDOWN: Id = 6;
pub const STATUS: Id = 7;
pub const NOTE: Id = 8;
pub const TODAY: Id = 9;
/// Text and shapes nothing refers to.
const DECOR: Id = 0;

/// The pending card's button widths, Snooze 5 min and Skip, and the footer's Quit, which sit
/// against a right edge.
const SNOOZE_W: i32 = 112;
const SKIP_W: i32 = 56;
const QUIT_W: i32 = 80;
/// The footer band's padding round its buttons.
const BAND_PAD: i32 = SPACE_2;

/// The countdown line: "Away " and " more and it counts" in `caption` `ink-muted`, the `m:ss`
/// in `caption-strong` `ink`.
pub fn countdown(left: Duration) -> Vec<Span> {
    let text = countdown_text(left);
    let (before, digits, after) = split_digits(&text);
    vec![
        Span::new(before, Style::Caption, Ink::Muted),
        Span::new(digits, Style::CaptionStrong, Ink::Main),
        Span::new(after, Style::Caption, Ink::Muted),
    ]
}

/// The Today count at the right of its row: "7 taken  2 skipped", the numbers in `body-strong`.
fn today_count(taken: u32, skipped: u32) -> Vec<Span> {
    vec![
        Span::new(&taken.to_string(), Style::BodyStrong, Ink::Main),
        Span::new(" taken  ", Style::Body, Ink::Muted),
        Span::new(&skipped.to_string(), Style::BodyStrong, Ink::Main),
        Span::new(" skipped", Style::Body, Ink::Muted),
    ]
}

fn text(id: Id, at: Place, text: &str, style: Style, ink: Ink) -> Item {
    Item::new(id, at, Kind::text(text, style, ink))
}

/// The flyout's items, top to bottom: the header, the Nudges row, the pending card while one is
/// pending, the Today row and the footer band, a `line` hairline between rows. 320 wide at
/// 96 DPI, `space-4` side margins; its height is the band's bottom.
pub fn items(c: &Content) -> Vec<Item> {
    let w = SIZE_FLYOUT;
    let pad = SPACE_4;
    let inner = w - 2 * pad;
    let body = Style::Body.line_height();
    let caption = Style::Caption.line_height();
    let mut out = Vec::new();
    let rule = |out: &mut Vec<Item>, y: i32| {
        out.push(Item::new(DECOR, Place::new(pad, y, inner, 1), Kind::Rule));
        y + 1
    };

    // Header: the status dot, "breakpoint" and the status line under it.
    let mut y = pad;
    out.push(Item::new(
        DECOR,
        Place::new(pad, y + (body - STATUS_DOT) / 2, STATUS_DOT, STATUS_DOT),
        Kind::Dot { on: c.on },
    ));
    let text_x = pad + STATUS_DOT + SPACE_3;
    out.push(text(
        DECOR,
        Place::at(text_x, y),
        "breakpoint",
        Style::BodyStrong,
        Ink::Main,
    ));
    y += body;
    out.push(text(
        STATUS,
        Place::new(text_x, y, w - pad - text_x, caption),
        &c.status,
        Style::Caption,
        Ink::Muted,
    ));
    y = rule(&mut out, y + caption + pad);

    // The Nudges row: the label, the note under it and the switch at the right.
    y += SPACE_3;
    let switch_x = w - pad - SWITCH_W;
    out.push(text(
        DECOR,
        Place::at(pad, y),
        "Nudges",
        Style::Body,
        Ink::Main,
    ));
    out.push(Item::new(
        SWITCH,
        Place::at(switch_x, y + (body + caption - SWITCH_H) / 2),
        Kind::Switch { on: c.on },
    ));
    out.push(text(
        NOTE,
        Place::new(pad, y + body, switch_x - SPACE_3 - pad, caption),
        &c.note,
        Style::Caption,
        Ink::Muted,
    ));
    y += body + caption + SPACE_3;

    // The pending card, `space-3` under the Nudges row.
    if let Some(p) = &c.pending {
        let top = y;
        let card = out.len();
        out.push(Item::new(
            DECOR,
            Place::default(),
            Kind::Card {
                rows: Vec::new(),
                inset: true,
            },
        ));
        let x = pad + SPACE_3;
        y += SPACE_3;
        let glyph = match p.nudge {
            Nudge::Stand => Glyph::Stand,
            Nudge::Pause => Glyph::Pause,
        };
        out.push(Item::new(
            DECOR,
            Place::new(x, y, SIZE_GLYPH, SIZE_GLYPH),
            Kind::Glyph(glyph),
        ));
        let line_x = x + SIZE_GLYPH + SPACE_2;
        out.push(text(
            DECOR,
            Place::at(line_x, y),
            &format!("{} pending", p.nudge.name()),
            Style::BodyStrong,
            Ink::Main,
        ));
        y += body.max(SIZE_GLYPH) + SPACE_1;
        out.push(Item::new(
            COUNTDOWN,
            Place::at(line_x, y),
            Kind::Spans {
                spans: countdown(p.left),
                right: false,
            },
        ));
        y += caption;
        if p.water {
            y += SPACE_2;
            out.push(Item::new(
                DECOR,
                Place::new(
                    line_x,
                    y + (body - SIZE_GLYPH_INLINE) / 2,
                    SIZE_GLYPH_INLINE,
                    SIZE_GLYPH_INLINE,
                ),
                Kind::Glyph(Glyph::Water),
            ));
            out.push(text(
                DECOR,
                Place::at(line_x + SIZE_GLYPH_INLINE + SPACE_2, y),
                WATER_LINE,
                Style::Body,
                Ink::Main,
            ));
            y += body;
        }
        // Snooze 5 min (secondary, while it may still be used) and Skip (ghost), at the right.
        y += SPACE_3;
        let skip_x = w - pad - SPACE_3 - SKIP_W;
        if p.snooze {
            out.push(Item::new(
                SNOOZE,
                Place::new(skip_x - SPACE_2 - SNOOZE_W, y, SNOOZE_W, 0),
                Kind::button("Snooze 5 min", ButtonKind::Secondary),
            ));
        }
        out.push(Item::new(
            SKIP,
            Place::new(skip_x, y, SKIP_W, 0),
            Kind::button("Skip", ButtonKind::Ghost),
        ));
        y += SIZE_BUTTON + SPACE_3;
        out[card].at = Place::new(pad, top, inner, y - top);
        y += SPACE_3;
    }
    y = rule(&mut out, y);

    // The Today row.
    y += SPACE_3;
    out.push(text(
        DECOR,
        Place::at(pad, y),
        "Today",
        Style::Body,
        Ink::Muted,
    ));
    out.push(Item::new(
        TODAY,
        Place::new(pad, y, inner, body),
        Kind::Spans {
            spans: today_count(c.taken, c.skipped),
            right: true,
        },
    ));
    y += body + SPACE_3;

    // The footer band: Settings… at the left, Quit at the right.
    let band_h = 2 * BAND_PAD + SIZE_BUTTON;
    out.push(Item::new(DECOR, Place::new(0, y, w, band_h), Kind::Band));
    let mut settings = Kind::button("Settings…", ButtonKind::Ghost);
    let mut quit = Kind::button("Quit", ButtonKind::Ghost);
    for (kind, mark) in [(&mut settings, Mark::Gear), (&mut quit, Mark::Power)] {
        if let Kind::Button { icon, .. } = kind {
            *icon = Some(mark);
        }
    }
    out.push(Item::new(
        SETTINGS,
        Place::at(BAND_PAD, y + BAND_PAD),
        settings,
    ));
    out.push(Item::new(
        QUIT,
        Place::new(w - BAND_PAD - QUIT_W, y + BAND_PAD, QUIT_W, 0),
        quit,
    ));
    out
}

// ---- Placement ----

/// The screen edge the taskbar sits on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Left,
    Top,
    Right,
    Bottom,
}

/// The flyout's top-left corner for a window of `size`, next to the notification icon at
/// `icon` and the taskbar at `taskbar` (its edge and rectangle; a bottom taskbar is assumed
/// when it is unknown), `gap` off the taskbar, inside `work` (the icon's monitor's work area)
/// and `gap` in from its sides.
///
/// Above a bottom taskbar and below a top one, the flyout's right edge lines up with the
/// icon's; beside a side taskbar, its bottom edge lines up with the icon's. An icon off the
/// taskbar (in the overflow) keeps the flyout clear of it too.
pub fn place(
    size: (i32, i32),
    icon: RECT,
    taskbar: Option<(Edge, RECT)>,
    work: RECT,
    gap: i32,
) -> POINT {
    let (w, h) = size;
    let (edge, bar) = taskbar.unwrap_or((Edge::Bottom, icon));
    let (x, y) = match edge {
        Edge::Bottom => (icon.right - w, icon.top.min(bar.top) - gap - h),
        Edge::Top => (icon.right - w, icon.bottom.max(bar.bottom) + gap),
        Edge::Left => (icon.right.max(bar.right) + gap, icon.bottom - h),
        Edge::Right => (icon.left.min(bar.left) - gap - w, icon.bottom - h),
    };
    // Inside the work area, `gap` in from its sides; a window too big keeps its top-left in.
    let clamp = |v: i32, low: i32, high: i32, len: i32| v.min(high - gap - len).max(low + gap);
    POINT {
        x: clamp(x, work.left, work.right, w).max(work.left),
        y: clamp(y, work.top, work.bottom, h).max(work.top),
    }
}

/// The taskbar's edge and rectangle, or `None` if Windows does not say.
fn taskbar() -> Option<(Edge, RECT)> {
    let mut data = APPBARDATA {
        cbSize: size_of::<APPBARDATA>() as u32,
        ..Default::default()
    };
    // SAFETY: `data` is a valid APPBARDATA with its size set.
    if unsafe { SHAppBarMessage(ABM_GETTASKBARPOS, &mut data) } == 0 {
        return None;
    }
    let edge = match data.uEdge {
        ABE_LEFT => Edge::Left,
        ABE_TOP => Edge::Top,
        ABE_RIGHT => Edge::Right,
        ABE_BOTTOM => Edge::Bottom,
        _ => return None,
    };
    Some((edge, data.rc))
}

/// The work area of the monitor `r` is on.
fn work_area(r: &RECT) -> RECT {
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: `info` is a valid MONITORINFO with its size set; `r` outlives the call.
    let _ = unsafe { GetMonitorInfoW(MonitorFromRect(r, MONITOR_DEFAULTTONEAREST), &mut info) };
    info.rcWork
}

// ---- The window ----

/// How often the watch looks for a click outside a flyout that has no focus.
const WATCH_EVERY_MS: u32 = 30;

thread_local! {
    /// The open flyout, if any.
    static OPEN: Cell<Option<Handle>> = const { Cell::new(None) };
    /// The icon it opened from, which it stays next to as its content changes.
    static ANCHOR: Cell<RECT> = const { Cell::new(RECT { left: 0, top: 0, right: 0, bottom: 0 }) };
    /// What it shows now.
    static SHOWN: RefCell<Option<Content>> = const { RefCell::new(None) };
    /// The app's latest content, which a click on the icon opens the flyout with.
    static LATEST: RefCell<Option<Content>> = const { RefCell::new(None) };
    /// When the flyout last closed because the user clicked outside it, and where the pointer
    /// was then.
    static LOST: Cell<Option<(Instant, POINT)>> = const { Cell::new(None) };
    /// The timer that watches for a click outside a flyout that has no focus; 0 when none.
    static WATCH: Cell<usize> = const { Cell::new(0) };
}

/// Whether `p` lies in `r`.
pub fn inside(r: &RECT, p: POINT) -> bool {
    p.x >= r.left && p.x < r.right && p.y >= r.top && p.y < r.bottom
}

fn open_handle() -> Option<Handle> {
    OPEN.get().filter(|h| h.is_open())
}

pub fn is_open() -> bool {
    open_handle().is_some()
}

/// Whether the flyout closed less than a double-click time ago because of a click outside it
/// on the icon at `icon`: a press on the icon now is that click, which only closes it.
pub fn lost_at(icon: &RECT) -> bool {
    // SAFETY: no pointers.
    let recent = Duration::from_millis(unsafe { GetDoubleClickTime() }.into());
    LOST.get()
        .is_some_and(|(at, p)| at.elapsed() < recent && inside(icon, p))
}

/// The app's content now: kept for the next open, and shown in the open flyout, if it is open.
pub fn show_content(content: Content) {
    update(&content);
    LATEST.set(Some(content));
}

/// Opens the flyout with the app's latest content next to the notification icon at `icon`
/// on screen, and activates it. Opened from the keyboard, it shows the focus ring on the
/// Nudges switch.
pub fn open(icon: RECT, keyboard: bool) {
    let Some(content) = LATEST.with_borrow(Clone::clone) else {
        return;
    };
    let Some(handle) = make(&content, icon) else {
        return;
    };
    handle.show();
    if !take_foreground(handle.hwnd()) {
        watch();
    }
    if keyboard {
        handle.focus(SWITCH);
    }
}

/// Makes `hwnd` the foreground window. When Windows refuses (the foreground lock), it joins
/// the foreground thread's input for a moment and asks again, as tray menus do. True when it
/// is the foreground window after.
fn take_foreground(hwnd: HWND) -> bool {
    // SAFETY: no pointers; the input is detached again before return.
    unsafe {
        let foreground = GetForegroundWindow();
        if foreground == hwnd {
            return true;
        }
        let theirs = GetWindowThreadProcessId(foreground, None);
        let ours = GetCurrentThreadId();
        if theirs != 0 && theirs != ours && AttachThreadInput(ours, theirs, true).as_bool() {
            let _ = BringWindowToTop(hwnd);
            let _ = SetForegroundWindow(hwnd);
            let _ = AttachThreadInput(ours, theirs, false);
        }
        GetForegroundWindow() == hwnd
    }
}

/// Starts the watch on a flyout that has no focus: it closes on the next click outside it.
fn watch() {
    stop_watch();
    // SAFETY: a thread timer with a plain callback.
    let id = unsafe { SetTimer(None, 0, WATCH_EVERY_MS, Some(watch_tick)) };
    WATCH.set(id);
}

fn stop_watch() {
    let id = WATCH.replace(0);
    if id != 0 {
        // SAFETY: the timer this thread made.
        let _ = unsafe { KillTimer(None, id) };
    }
}

/// Whether a mouse button is `down` at `at`, outside the flyout's rectangle `flyout`.
fn outside_press(down: bool, at: POINT, flyout: &RECT) -> bool {
    down && !inside(flyout, at)
}

unsafe extern "system" fn watch_tick(_: HWND, _: u32, _: usize, _: u32) {
    let Some(handle) = open_handle() else {
        stop_watch();
        return;
    };
    // SAFETY: no pointers but the valid out-pointer.
    unsafe {
        if GetForegroundWindow() == handle.hwnd() {
            // A click on it gave it the focus: losing activation closes it from now on.
            stop_watch();
            return;
        }
        let down = [VK_LBUTTON, VK_RBUTTON, VK_MBUTTON]
            .iter()
            .any(|vk| GetAsyncKeyState(vk.0.into()) < 0);
        let mut at = POINT::default();
        let _ = GetCursorPos(&mut at);
        if outside_press(down, at, &handle.window_rect()) {
            lost(at);
        }
    }
}

/// Closes the flyout after a click outside it, with the pointer at `at`.
fn lost(at: POINT) {
    LOST.set(Some((Instant::now(), at)));
    close();
}

/// Makes the flyout window, placed and hidden, in place of any open one.
fn make(content: &Content, icon: RECT) -> Option<Handle> {
    close();
    let spec = Spec {
        caption: "breakpoint".into(),
        width: SIZE_FLYOUT,
        height: 0,
        items: items(content),
        // On the icon's monitor, so the window takes that monitor's DPI; placed below.
        at: Some(POINT {
            x: icon.left,
            y: icon.top,
        }),
        owner: None,
        frame: Frame::Popup,
    };
    let handle = window::open(spec, on_event).ok()?;
    OPEN.set(Some(handle));
    ANCHOR.set(icon);
    SHOWN.set(Some(content.clone()));
    put(handle);
    Some(handle)
}

/// Moves the open flyout next to its icon, for its size now.
fn put(handle: Handle) {
    let r = handle.window_rect();
    let icon = ANCHOR.get();
    // SAFETY: no pointers.
    let dpi = unsafe { GetDpiForWindow(handle.hwnd()) };
    let at = place(
        (r.right - r.left, r.bottom - r.top),
        icon,
        taskbar(),
        work_area(&icon),
        px(SPACE_3, dpi),
    );
    handle.move_to(at.x, at.y);
}

/// Shows `content` in the open flyout, if it is open: the countdown changes in place; any
/// other change lays the flyout out again next to its icon, keeping the focus.
fn update(content: &Content) {
    let Some(handle) = open_handle() else {
        return;
    };
    let shown = SHOWN.take();
    SHOWN.set(Some(content.clone()));
    match shown {
        Some(s) if s == *content => {}
        Some(s) if s.same_but_countdown(content) => {
            if let Some(p) = &content.pending {
                handle.set_spans(COUNTDOWN, countdown(p.left));
            }
        }
        _ => {
            handle.replace(items(content));
            put(handle);
        }
    }
}

/// Closes the flyout if it is open.
pub fn close() {
    stop_watch();
    if let Some(handle) = OPEN.take() {
        handle.close();
    }
    SHOWN.set(None);
}

/// What the user did in the flyout. Every action closes it, as a menu closes.
fn on_event(handle: Handle, event: Event) {
    let command = match event {
        Event::Changed(SWITCH, Value::On(_)) => Some(Command::Switch),
        Event::Pressed(SNOOZE) => Some(Command::Snooze),
        Event::Pressed(SKIP) => Some(Command::Skip),
        Event::Pressed(SETTINGS) => Some(Command::Settings),
        Event::Pressed(QUIT) => Some(Command::Quit),
        Event::Close => None,
        Event::Lost if OPEN.get() == Some(handle) => {
            let mut at = POINT::default();
            // SAFETY: a valid out-pointer.
            let _ = unsafe { GetCursorPos(&mut at) };
            lost(at);
            return;
        }
        Event::Lost => None,
        _ => return,
    };
    if OPEN.get() == Some(handle) {
        close();
    } else {
        handle.close();
    }
    if let Some(command) = command {
        tray::push(command);
    }
}

#[cfg(test)]
mod tests;
