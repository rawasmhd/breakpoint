//! The nudge popup window (SPEC §6, DESIGN.md §6).
//!
//! One window for the life of the app, hidden between occurrences (SPEC §6: at most one popup on
//! screen). It sits bottom-right of the primary monitor's work area, just above the tray, always
//! on top, and never takes focus: `WS_EX_NOACTIVATE`, `SW_SHOWNOACTIVATE` and `MA_NOACTIVATE`.
//!
//! There is no caption and there are no child windows. The whole client area is painted in
//! `WM_PAINT`, double-buffered: shapes with GDI+ ([`super::paint`]), text with GDI in the theme
//! fonts. The header, the close ×, Snooze 5 min and Skip are painted too, with hover and press
//! fills from mouse tracking. Every colour, size, font and timing comes from [`super::theme`].
//!
//! Button presses are posted to the owner window as messages, so the app reacts from its own
//! window procedure rather than from inside this one. Water alone and the bad-config notice use
//! the same window.

use std::cell::RefCell;
use std::time::{Duration, Instant};

use breakpoint::core::scheduler::{
    Nudge, Prompt, WATER_ALONE_FOR, WATER_LINE, WATER_TITLE, countdown_text,
};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmSetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateSolidBrush, DeleteDC,
    DeleteObject, EndPaint, FillRect, GetDC, GetMonitorInfoW, GetTextExtentPoint32W,
    GetTextMetricsW, HDC, HMONITOR, InvalidateRect, MONITOR_DEFAULTTOPRIMARY, MONITORINFO,
    MonitorFromPoint, PAINTSTRUCT, ReleaseDC, SRCCOPY, SelectObject, SetBkMode, SetTextAlign,
    SetTextColor, TA_BASELINE, TA_LEFT, TEXTMETRICW, TRANSPARENT, TextOutW,
};
use windows::Win32::Media::Audio::{PlaySoundW, SND_ALIAS, SND_ASYNC};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::SystemServices::MK_LBUTTON;
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent,
    VK_LBUTTON,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CS_DROPSHADOW, CreateWindowExW, DefWindowProcW, HWND_TOPMOST, IDC_ARROW, IsWindowVisible,
    KillTimer, LWA_ALPHA, LoadCursorW, MA_NOACTIVATE, PostMessageW, RegisterClassW, SW_HIDE,
    SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SWP_SHOWWINDOW,
    SetLayeredWindowAttributes, SetTimer, SetWindowPos, ShowWindow,
    USER_DEFAULT_SCREEN_DPI as BASE_DPI, WINDOW_EX_STYLE, WM_APP, WM_CAPTURECHANGED, WM_CLOSE,
    WM_DPICHANGED, WM_ERASEBKGND, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEACTIVATE, WM_MOUSEMOVE,
    WM_PAINT, WM_SETTINGCHANGE, WM_TIMER, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};
use windows::core::{Result, w};

use super::paint::{self, Box2, Canvas, FRAME, Glyph, HOVER, PRESSED, REST};
use super::theme::{
    self, BAR_GAP, CLOSE_HIT, CLOSE_OVERHANG, FADE_IN, FADE_OUT, FontCache, Fonts, HAIRLINE,
    POPUP_HEADER, PRESS_DROP, Palette, RADIUS_SM, SIZE_BAR, SIZE_BUTTON, SIZE_GLYPH,
    SIZE_GLYPH_INLINE, SIZE_MARK, SIZE_POPUP, SPACE_1, SPACE_2, SPACE_3, SPACE_4, Style, Theme,
    WATER_ALONE_PAD_X, WATER_ALONE_PAD_Y, px,
};

/// Posted to the owner window when the user presses Skip.
pub const WM_POPUP_SKIP: u32 = WM_APP + 1;
/// Posted to the owner window when the user presses Snooze 5 min.
pub const WM_POPUP_SNOOZE: u32 = WM_APP + 2;

/// Hides Water alone once it has been on screen for [`WATER_ALONE_FOR`].
const WATER_TIMER: usize = 1;
/// Runs the fade and the button fills, one repaint per [`FRAME`], while any of them moves.
const FRAME_TIMER: usize = 2;

/// The bad-config notice (SPEC §10.1).
const NOTICE: &str = "breakpoint couldn't read config.json and is using the defaults. \
                      Saving from Settings will replace the file.";
/// The app's name in the popup header, lowercase (DESIGN.md §2).
const APP_NAME: &str = "breakpoint";
const SNOOZE_LABEL: &str = "Snooze 5 min";
const SKIP_LABEL: &str = "Skip";

const EX_STYLE: WINDOW_EX_STYLE =
    WINDOW_EX_STYLE(WS_EX_TOPMOST.0 | WS_EX_TOOLWINDOW.0 | WS_EX_NOACTIVATE.0 | WS_EX_LAYERED.0);

// ---- What the popup holds, and where it goes ----

/// What the popup shows.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Content {
    Prompt {
        nudge: Nudge,
        title: &'static str,
        line: String,
        water: bool,
        snooze: bool,
    },
    WaterAlone,
    Notice,
}

/// The parts that answer the pointer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Part {
    Close,
    Snooze,
    Skip,
}

impl Part {
    const ALL: [Part; 3] = [Part::Close, Part::Snooze, Part::Skip];

    fn label(self) -> Option<&'static str> {
        match self {
            Part::Close => None,
            Part::Snooze => Some(SNOOZE_LABEL),
            Part::Skip => Some(SKIP_LABEL),
        }
    }
}

/// Whose colour a text item takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ink {
    Main,
    Muted,
}

/// One thing to paint, in client pixels at the layout's DPI.
#[derive(Debug, Clone, PartialEq)]
enum Item {
    /// The breakpoint dot in the header.
    Dot(RECT),
    Glyph(Glyph, RECT),
    /// One line of text; `top` is the top of its line box, `Style::line_height` tall.
    Text {
        style: Style,
        ink: Ink,
        x: i32,
        top: i32,
        text: String,
    },
    /// The countdown bar's track; the fill follows the away time at paint.
    Bar(RECT),
    /// The countdown line's box; its text follows the away time at paint.
    Countdown {
        x: i32,
        top: i32,
    },
    Button(Part, RECT),
}

#[derive(Debug, Clone, PartialEq)]
struct Layout {
    width: i32,
    height: i32,
    items: Vec<Item>,
}

impl Layout {
    fn hit(&self, x: i32, y: i32) -> Option<Part> {
        self.items.iter().find_map(|item| match item {
            Item::Button(part, r) if x >= r.left && x < r.right && y >= r.top && y < r.bottom => {
                Some(*part)
            }
            _ => None,
        })
    }
}

/// Text widths, so the layout can wrap lines and size buttons.
trait Measure {
    fn width(&self, style: Style, text: &str) -> i32;
}

fn rect(x: i32, y: i32, w: i32, h: i32) -> RECT {
    RECT {
        left: x,
        top: y,
        right: x + w,
        bottom: y + h,
    }
}

/// `text` broken at spaces into lines no wider than `width`. A word wider than `width` gets a
/// line of its own.
fn wrap(text: &str, width: i32, fits: impl Fn(&str) -> i32) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        let longer = if line.is_empty() {
            word.to_owned()
        } else {
            format!("{line} {word}")
        };
        if line.is_empty() || fits(&longer) <= width {
            line = longer;
        } else {
            lines.push(std::mem::replace(&mut line, word.to_owned()));
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// Lays out `content` at `dpi` (DESIGN.md §6, Popup).
fn layout(content: &Content, dpi: u32, m: &impl Measure) -> Layout {
    let s = |v: i32| px(v, dpi);
    let lh = |style: Style| s(style.line_height());
    let width = s(SIZE_POPUP);
    let pad = s(SPACE_4);
    let mut items = Vec::new();

    let text = |items: &mut Vec<Item>, style, ink, x, top, text: &str| {
        items.push(Item::Text {
            style,
            ink,
            x,
            top,
            text: text.to_owned(),
        })
    };
    // Lines of `body` from `top`, wrapped to the space right of `x`; returns the height used.
    let paragraph = |items: &mut Vec<Item>, ink, x: i32, top: i32, words: &str| {
        let lines = wrap(words, width - pad - x, |t| m.width(Style::Body, t));
        for (i, line) in lines.iter().enumerate() {
            text(
                items,
                Style::Body,
                ink,
                x,
                top + i as i32 * lh(Style::Body),
                line,
            );
        }
        lines.len() as i32 * lh(Style::Body)
    };
    // The header row at `top`: dot, "breakpoint", close ×.
    let header = |items: &mut Vec<Item>, top: i32| {
        let row = s(POPUP_HEADER);
        let mark = s(SIZE_MARK);
        items.push(Item::Dot(rect(pad, top + (row - mark) / 2, mark, mark)));
        let x = pad + mark + s(SPACE_2);
        text(
            items,
            Style::Caption,
            Ink::Muted,
            x,
            top + (row - lh(Style::Caption)) / 2,
            APP_NAME,
        );
        let hit = s(CLOSE_HIT);
        let right = width - pad + s(CLOSE_OVERHANG);
        items.push(Item::Button(
            Part::Close,
            rect(right - hit, top + (row - hit) / 2, hit, hit),
        ));
        top + row
    };

    let height = match content {
        Content::WaterAlone => {
            let (pad_x, pad_y) = (s(WATER_ALONE_PAD_X), s(WATER_ALONE_PAD_Y));
            let glyph = s(SIZE_GLYPH);
            let row = glyph.max(lh(Style::BodyStrong));
            items.push(Item::Glyph(
                Glyph::Water,
                rect(pad_x, pad_y + (row - glyph) / 2, glyph, glyph),
            ));
            text(
                &mut items,
                Style::BodyStrong,
                Ink::Main,
                pad_x + glyph + s(SPACE_2),
                pad_y + (row - lh(Style::BodyStrong)) / 2,
                WATER_TITLE,
            );
            pad_y + row + pad_y
        }
        Content::Notice => {
            let top = header(&mut items, pad) + s(SPACE_3);
            top + paragraph(&mut items, Ink::Main, pad, top, NOTICE) + pad
        }
        Content::Prompt {
            nudge,
            title,
            line,
            water,
            snooze,
        } => {
            let mut y = header(&mut items, pad) + s(SPACE_3);

            // Glyph and title, the glyph centred on the title's line.
            let glyph = s(SIZE_GLYPH);
            let row = glyph.max(lh(Style::Title));
            let kind = match nudge {
                Nudge::Stand => Glyph::Stand,
                Nudge::Pause => Glyph::Pause,
            };
            items.push(Item::Glyph(
                kind,
                rect(pad, y + (row - glyph) / 2, glyph, glyph),
            ));
            let text_x = pad + glyph + s(SPACE_2);
            let title_top = y + (row - lh(Style::Title)) / 2;
            text(
                &mut items,
                Style::Title,
                Ink::Main,
                text_x,
                title_top,
                title,
            );
            y += row + s(SPACE_1);

            y += paragraph(&mut items, Ink::Muted, text_x, y, line);

            if *water {
                y += s(SPACE_2);
                let glyph = s(SIZE_GLYPH_INLINE);
                let row = glyph.max(lh(Style::Body));
                items.push(Item::Glyph(
                    Glyph::Water,
                    rect(text_x, y + (row - glyph) / 2, glyph, glyph),
                ));
                text(
                    &mut items,
                    Style::Body,
                    Ink::Main,
                    text_x + glyph + s(SPACE_2),
                    y + (row - lh(Style::Body)) / 2,
                    WATER_LINE,
                );
                y += row;
            }

            y += s(SPACE_4);
            items.push(Item::Bar(rect(pad, y, width - 2 * pad, s(SIZE_BAR))));
            y += s(SIZE_BAR) + s(BAR_GAP);
            items.push(Item::Countdown { x: pad, top: y });
            y += lh(Style::Caption) + s(SPACE_4);

            // Buttons right-aligned, Skip last: [Snooze 5 min] [Skip].
            let mut right = width - pad;
            let shown: &[Part] = if *snooze {
                &[Part::Skip, Part::Snooze]
            } else {
                &[Part::Skip]
            };
            let mut buttons = Vec::new();
            for &part in shown {
                let label = part.label().unwrap_or_default();
                let w = m.width(Style::BodyStrong, label) + 2 * (s(SPACE_3) + s(HAIRLINE));
                buttons.push(Item::Button(part, rect(right - w, y, w, s(SIZE_BUTTON))));
                right -= w + s(SPACE_2);
            }
            items.extend(buttons.into_iter().rev());
            y + s(SIZE_BUTTON) + pad
        }
    };
    Layout {
        width,
        height,
        items,
    }
}

/// The countdown line cut round its `m:ss`: the words before, the digits, the words after.
fn split_digits(text: &str) -> (&str, &str, &str) {
    let start = text
        .find(|c: char| c.is_ascii_digit())
        .unwrap_or(text.len());
    let len = text[start..]
        .find(|c: char| !(c.is_ascii_digit() || c == ':'))
        .unwrap_or(text.len() - start);
    (
        &text[..start],
        &text[start..start + len],
        &text[start + len..],
    )
}

/// How full the countdown bar is: the away time so far over the length.
fn bar_fraction(left: Duration, length: Duration) -> f32 {
    if length.is_zero() {
        return 1.0;
    }
    (1.0 - left.as_secs_f32() / length.as_secs_f32()).clamp(0.0, 1.0)
}

// ---- Window state ----

struct State {
    owner: HWND,
    content: Content,
    /// The full away length of the prompt on screen, and how much of it is left.
    length: Duration,
    left: Duration,
    dpi: u32,
    layout: Layout,
    theme: Theme,
    fonts: FontCache,
    /// No DWM round corners (Windows 10): paint a hairline outline instead.
    outline: bool,
    pointer: Pointer,
    tracking: bool,
    /// Fill level per part, in [`Part::ALL`] order: 0 rest, 1 hover, 2 press.
    fills: [f32; 3],
    /// Window opacity, 0 to 1, and where the fade is heading.
    alpha: f32,
    fade_to: f32,
    last_frame: Instant,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

/// Runs `f` on the state, unless there is none or it is already in use (a message sent while
/// another handler runs). Never call into Win32 from `f` in a way that sends this window a
/// message.
fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    STATE.with(|cell| cell.try_borrow_mut().ok()?.as_mut().map(f))
}

/// Which part the pointer is over, and which part a press started on.
///
/// The popup never activates, so after `SetCapture` Windows still sends the release to the
/// window under the pointer once it leaves the popup. A press whose release went elsewhere ends
/// at the next sign that the button is up, so it can never act later.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct Pointer {
    hot: Option<Part>,
    pressed: Option<Part>,
}

impl Pointer {
    /// The pointer moved over `hit`, with the left button `down` or not. True if anything changed.
    fn moved(&mut self, hit: Option<Part>, down: bool) -> bool {
        let before = *self;
        if !down {
            self.pressed = None;
        }
        self.hot = hit;
        *self != before
    }

    /// The pointer left the popup, with the left button `down` or not.
    fn left(&mut self, down: bool) {
        self.hot = None;
        if !down {
            self.pressed = None;
        }
    }

    fn down(&mut self, hit: Option<Part>) {
        self.hot = hit;
        self.pressed = hit;
    }

    /// The button came up over `hit`: the part to act on, only if the press started on it.
    fn up(&mut self, hit: Option<Part>) -> Option<Part> {
        self.hot = hit;
        self.pressed.take().filter(|&p| Some(p) == hit)
    }

    fn cancel(&mut self) {
        self.pressed = None;
    }

    fn target(&self, part: Part) -> f32 {
        match (self.hot == Some(part), self.pressed) {
            (true, Some(p)) if p == part => PRESSED,
            (true, None) => HOVER,
            _ => REST,
        }
    }
}

impl State {
    fn target(&self, part: Part) -> f32 {
        self.pointer.target(part)
    }

    fn moving(&self) -> bool {
        self.alpha != self.fade_to
            || Part::ALL
                .iter()
                .zip(self.fills)
                .any(|(&p, level)| level != self.target(p))
    }

    /// Lays out again for `dpi` and returns the window's place: bottom-right of the primary
    /// monitor's work area, `space-4` in from its corner.
    fn relayout(&mut self, dpi: u32) -> RECT {
        self.dpi = dpi;
        let fonts = self.fonts.at(dpi);
        self.layout = with_screen_measure(fonts, |m| layout(&self.content, dpi, m));
        let work = work_area();
        let gap = px(SPACE_4, dpi);
        let (w, h) = (self.layout.width, self.layout.height);
        rect(work.right - w - gap, work.bottom - h - gap, w, h)
    }
}

pub struct Popup {
    hwnd: HWND,
}

impl Popup {
    /// Creates the popup hidden; `owner` receives its button presses as `WM_POPUP_*` messages.
    pub fn new(owner: HWND) -> Result<Self> {
        paint::startup();
        // SAFETY: plain Win32 window creation; every handle passed is one created just before.
        unsafe {
            let instance = GetModuleHandleW(None)?.into();
            let class = w!("breakpoint.popup");
            RegisterClassW(&WNDCLASSW {
                style: CS_DROPSHADOW,
                lpfnWndProc: Some(wndproc),
                hInstance: instance,
                lpszClassName: class,
                hCursor: LoadCursorW(None, IDC_ARROW)?,
                ..Default::default()
            });
            // Created at the primary monitor's origin so it starts with that monitor's DPI.
            let hwnd = CreateWindowExW(
                EX_STYLE,
                class,
                w!("breakpoint"),
                WS_POPUP,
                0,
                0,
                0,
                0,
                None,
                None,
                Some(instance),
                None,
            )?;
            // A layered window shows nothing until its alpha is set.
            let _ = SetLayeredWindowAttributes(hwnd, COLORREF::default(), 0, LWA_ALPHA);
            // Round corners on Windows 11. Windows 10 does not know the attribute and says no.
            let round = DwmSetWindowAttribute(
                hwnd,
                DWMWA_WINDOW_CORNER_PREFERENCE,
                &DWMWCP_ROUND as *const _ as _,
                size_of_val(&DWMWCP_ROUND) as u32,
            );
            let content = Content::Notice;
            let mut fonts = FontCache::new();
            let dpi = primary_dpi();
            let layout = with_screen_measure(fonts.at(dpi), |m| layout(&content, dpi, m));
            STATE.set(Some(State {
                owner,
                content,
                length: Duration::ZERO,
                left: Duration::ZERO,
                dpi,
                layout,
                theme: theme::app_theme(),
                fonts,
                outline: round.is_err(),
                pointer: Pointer::default(),
                tracking: false,
                fills: [REST; 3],
                alpha: 0.0,
                fade_to: 0.0,
                last_frame: Instant::now(),
            }));
            Ok(Self { hwnd })
        }
    }

    /// Shows `prompt` on top without taking focus, with the default notification sound. `left`
    /// is the away time still needed (SPEC §6).
    pub fn show(&self, prompt: &Prompt, left: Duration) {
        self.raise(
            Content::Prompt {
                nudge: prompt.nudge,
                title: prompt.title,
                line: prompt.line.clone(),
                water: prompt.water,
                snooze: prompt.snooze,
            },
            prompt.length,
            left,
        );
        // SAFETY: a static alias name; no other pointers.
        let _ = unsafe { PlaySoundW(w!("Notification.Default"), None, SND_ALIAS | SND_ASYNC) };
    }

    /// Shows Water alone (SPEC §5.4): one row, no buttons, silent, hiding itself after
    /// [`WATER_ALONE_FOR`].
    pub fn water(&self) {
        self.raise(Content::WaterAlone, Duration::ZERO, Duration::ZERO);
        // SAFETY: `self.hwnd` is the live popup window.
        unsafe {
            SetTimer(
                Some(self.hwnd),
                WATER_TIMER,
                WATER_ALONE_FOR.as_millis() as u32,
                None,
            );
        }
    }

    /// Shows the bad-config notice in the popup's place: no buttons, silent, until closed with
    /// ×. A nudge shown later replaces it.
    pub fn notice(&self) {
        self.raise(Content::Notice, Duration::ZERO, Duration::ZERO);
    }

    /// Lays the popup out afresh for `content` and puts it on top without activating it,
    /// fading in when it was hidden or fading out.
    fn raise(&self, content: Content, length: Duration, left: Duration) {
        let dpi = primary_dpi();
        let animate = theme::animations_on();
        let visible = self.is_visible();
        let Some((place, alpha)) = with_state(|s| {
            s.content = content;
            s.length = length;
            s.left = left;
            // The theme may have changed while hidden.
            s.theme = theme::app_theme();
            if !visible {
                s.alpha = if animate { 0.0 } else { 1.0 };
                s.pointer = Pointer::default();
                s.fills = [REST; 3];
            }
            s.fade_to = 1.0;
            if !animate {
                s.alpha = 1.0;
            }
            s.last_frame = Instant::now();
            // The work area or the scale may have changed while hidden.
            (s.relayout(dpi), s.alpha)
        }) else {
            return;
        };
        // SAFETY: `self.hwnd` is the live popup window; no state borrow is held.
        unsafe {
            // Whatever comes next replaces Water alone, so its timer must not hide it.
            let _ = KillTimer(Some(self.hwnd), WATER_TIMER);
            set_alpha(self.hwnd, alpha);
            move_to(self.hwnd, place);
            let _ = InvalidateRect(Some(self.hwnd), None, false);
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
            start_frames(self.hwnd);
        }
    }

    /// Updates the away time still needed; the countdown and its bar repaint.
    pub fn set_countdown(&self, left: Duration) {
        with_state(|s| s.left = left);
        // SAFETY: `self.hwnd` is the live popup window.
        let _ = unsafe { InvalidateRect(Some(self.hwnd), None, false) };
    }

    pub fn is_visible(&self) -> bool {
        // SAFETY: no pointers.
        unsafe { IsWindowVisible(self.hwnd).as_bool() }
    }

    /// Fades the popup out and hides it.
    pub fn hide(&self) {
        hide(self.hwnd);
    }
}

// ---- Win32 helpers ----

fn primary() -> HMONITOR {
    // SAFETY: no pointers; the primary monitor always contains the origin.
    unsafe { MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY) }
}

fn primary_dpi() -> u32 {
    let (mut x, mut y) = (0, 0);
    // SAFETY: `x` and `y` are valid out-pointers for the call.
    match unsafe { GetDpiForMonitor(primary(), MDT_EFFECTIVE_DPI, &mut x, &mut y) } {
        Ok(()) if x > 0 => x,
        _ => BASE_DPI,
    }
}

fn work_area() -> RECT {
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: `info` is a valid MONITORINFO with its size set.
    let _ = unsafe { GetMonitorInfoW(primary(), &mut info) };
    info.rcWork
}

/// Measures text with `fonts` on a screen device context.
struct GdiMeasure<'a> {
    dc: HDC,
    fonts: &'a Fonts,
}

impl Measure for GdiMeasure<'_> {
    fn width(&self, style: Style, text: &str) -> i32 {
        let wide: Vec<u16> = text.encode_utf16().collect();
        let mut size = SIZE::default();
        // SAFETY: `self.dc` is live for the measure's life; `wide` and `size` outlive the call.
        unsafe {
            let old = SelectObject(self.dc, self.fonts.get(style).into());
            let _ = GetTextExtentPoint32W(self.dc, &wide, &mut size);
            SelectObject(self.dc, old);
        }
        size.cx
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

unsafe fn set_alpha(hwnd: HWND, alpha: f32) {
    let byte = (alpha.clamp(0.0, 1.0) * f32::from(u8::MAX)).round() as u8;
    // SAFETY: per the caller, `hwnd` is the live popup window.
    let _ = unsafe { SetLayeredWindowAttributes(hwnd, COLORREF::default(), byte, LWA_ALPHA) };
}

unsafe fn move_to(hwnd: HWND, place: RECT) {
    // SAFETY: per the caller, `hwnd` is live and no state borrow is held: this may send
    // WM_DPICHANGED, which lays out again.
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

unsafe fn start_frames(hwnd: HWND) {
    // SAFETY: per the caller, `hwnd` is the live popup window.
    unsafe { SetTimer(Some(hwnd), FRAME_TIMER, FRAME.as_millis() as u32, None) };
}

/// Fades out, or hides at once when Windows animation is off. The occurrence is untouched.
fn hide(hwnd: HWND) {
    // SAFETY: `hwnd` is the live popup window.
    unsafe {
        let _ = KillTimer(Some(hwnd), WATER_TIMER);
        if !IsWindowVisible(hwnd).as_bool() {
            return;
        }
        if theme::animations_on() {
            with_state(|s| {
                s.fade_to = 0.0;
                s.pointer.cancel();
                s.last_frame = Instant::now();
            });
            start_frames(hwnd);
        } else {
            finish_hide(hwnd);
        }
    }
}

unsafe fn finish_hide(hwnd: HWND) {
    with_state(|s| {
        s.alpha = 0.0;
        s.fade_to = 0.0;
        s.pointer = Pointer::default();
        s.tracking = false;
        s.fills = [REST; 3];
    });
    // SAFETY: per the caller, `hwnd` is the live popup window; no state borrow is held.
    unsafe {
        let _ = KillTimer(Some(hwnd), FRAME_TIMER);
        let _ = ShowWindow(hwnd, SW_HIDE);
        set_alpha(hwnd, 0.0);
    }
}

/// One frame of the fade and the button fills.
unsafe fn frame(hwnd: HWND) {
    let animate = theme::animations_on();
    let Some((alpha, done, gone)) = with_state(|s| {
        let now = Instant::now();
        let dt = now - s.last_frame;
        s.last_frame = now;
        for (i, &part) in Part::ALL.iter().enumerate() {
            s.fills[i] = paint::step_fill(s.fills[i], s.target(part), dt, animate);
        }
        let span = if s.fade_to > s.alpha {
            FADE_IN
        } else {
            FADE_OUT
        };
        let step = if animate {
            dt.as_secs_f32() / span.as_secs_f32()
        } else {
            1.0
        };
        s.alpha = if s.fade_to > s.alpha {
            (s.alpha + step).min(s.fade_to)
        } else {
            (s.alpha - step).max(s.fade_to)
        };
        (s.alpha, !s.moving(), s.fade_to == 0.0 && s.alpha == 0.0)
    }) else {
        return;
    };
    // SAFETY: per the caller, `hwnd` is the live popup window; no state borrow is held.
    unsafe {
        if gone {
            finish_hide(hwnd);
            return;
        }
        set_alpha(hwnd, alpha);
        let _ = InvalidateRect(Some(hwnd), None, false);
        if done {
            let _ = KillTimer(Some(hwnd), FRAME_TIMER);
        }
    }
}

// ---- Paint ----

unsafe fn paint(hwnd: HWND) {
    let mut ps = PAINTSTRUCT::default();
    // SAFETY: Begin/EndPaint pair on the live window; every GDI object made here is freed here.
    unsafe {
        let dc = BeginPaint(hwnd, &mut ps);
        with_state(|s| {
            let (w, h) = (s.layout.width, s.layout.height);
            let mem = CreateCompatibleDC(Some(dc));
            let bitmap = CreateCompatibleBitmap(dc, w, h);
            let old = SelectObject(mem, bitmap.into());
            // The ground in GDI first, so text stays readable even if GDI+ is not up.
            let ground = CreateSolidBrush(s.theme.palette().surface_raised);
            FillRect(mem, &rect(0, 0, w, h), ground);
            let _ = DeleteObject(ground.into());
            paint_shapes(s, mem);
            paint_text(s, mem);
            let _ = BitBlt(dc, 0, 0, w, h, Some(mem), 0, 0, SRCCOPY);
            SelectObject(mem, old);
            let _ = DeleteObject(bitmap.into());
            let _ = DeleteDC(mem);
        });
        let _ = EndPaint(hwnd, &ps);
    }
}

fn fill_index(part: Part) -> usize {
    Part::ALL
        .iter()
        .position(|&p| p == part)
        .unwrap_or_default()
}

fn glyph_colour(p: &Palette, glyph: Glyph) -> COLORREF {
    match glyph {
        Glyph::Stand => p.stand,
        Glyph::Pause => p.pause,
        Glyph::Water => p.water,
    }
}

/// The ground, the outline on Windows 10, the dot, glyphs, the bar and the button fills.
unsafe fn paint_shapes(s: &State, dc: HDC) {
    let p = s.theme.palette();
    let scale = |v: i32| px(v, s.dpi) as f32;
    // SAFETY: per the caller, `dc` is a live memory DC; nothing else draws on it meanwhile.
    let Some(canvas) = (unsafe { Canvas::new(dc) }) else {
        return;
    };
    let all = Box2::from(rect(0, 0, s.layout.width, s.layout.height));
    if s.outline {
        canvas.fill_rect(p.line, all);
        canvas.fill_rect(p.surface_raised, all.inset(scale(HAIRLINE)));
    } else {
        canvas.fill_rect(p.surface_raised, all);
    }
    let bar_colour = match &s.content {
        Content::Prompt {
            nudge: Nudge::Stand,
            ..
        } => p.stand,
        _ => p.pause,
    };
    for item in &s.layout.items {
        match item {
            Item::Dot(r) => {
                let b = Box2::from(*r);
                canvas.fill_circle(p.dot, b.x + b.w / 2.0, b.y + b.h / 2.0, b.w / 2.0);
            }
            Item::Glyph(glyph, r) => canvas.glyph(*glyph, glyph_colour(p, *glyph), (*r).into()),
            Item::Bar(r) => {
                let track = Box2::from(*r);
                let round = track.h / 2.0;
                canvas.fill_round_rect(p.surface_sunken, track, round);
                let filled = Box2 {
                    w: track.w * bar_fraction(s.left, s.length),
                    ..track
                };
                if filled.w > 0.0 {
                    canvas.fill_round_rect(bar_colour, filled, round);
                }
            }
            Item::Button(part, r) => {
                let b = Box2::from(*r);
                let level = s.fills[fill_index(*part)];
                let fill = paint::fill_colour(p, level);
                let radius = scale(RADIUS_SM);
                match part {
                    Part::Snooze => {
                        let border = scale(HAIRLINE);
                        canvas.fill_round_rect(p.line_strong, b, radius);
                        canvas.fill_round_rect(fill, b.inset(border), radius - border);
                    }
                    Part::Skip | Part::Close => {
                        if level > REST {
                            canvas.fill_round_rect(fill, b, radius);
                        }
                    }
                }
                if *part == Part::Close {
                    let ink = paint::mix(p.ink_muted, p.ink, level.min(HOVER));
                    canvas.close_mark(ink, b, s.dpi as f32 / BASE_DPI as f32);
                }
            }
            Item::Text { .. } | Item::Countdown { .. } => {}
        }
    }
}

/// Every line of text, in the theme fonts, on the line boxes the layout gave.
unsafe fn paint_text(s: &mut State, dc: HDC) {
    let p = *s.theme.palette();
    let dpi = s.dpi;
    let fonts = s.fonts.at(dpi);
    // SAFETY: per the caller, `dc` is a live memory DC; fonts outlive the calls.
    unsafe {
        SetBkMode(dc, TRANSPARENT);
        SetTextAlign(dc, TA_LEFT | TA_BASELINE);
        // Draws `text` on the line box at `top`; returns the x after it.
        let draw = |style: Style, colour: COLORREF, x: i32, top: i32, text: &str| -> i32 {
            let wide: Vec<u16> = text.encode_utf16().collect();
            let old = SelectObject(dc, fonts.get(style).into());
            let mut tm = TEXTMETRICW::default();
            let _ = GetTextMetricsW(dc, &mut tm);
            let baseline = top + (px(style.line_height(), dpi) - tm.tmHeight) / 2 + tm.tmAscent;
            SetTextColor(dc, colour);
            let _ = TextOutW(dc, x, baseline, &wide);
            let mut size = SIZE::default();
            let _ = GetTextExtentPoint32W(dc, &wide, &mut size);
            SelectObject(dc, old);
            x + size.cx
        };
        let ink = |i: Ink| match i {
            Ink::Main => p.ink,
            Ink::Muted => p.ink_muted,
        };
        for item in &s.layout.items {
            match item {
                Item::Text {
                    style,
                    ink: i,
                    x,
                    top,
                    text,
                } => {
                    draw(*style, ink(*i), *x, *top, text);
                }
                Item::Countdown { x, top } => {
                    let line = countdown_text(s.left);
                    let (before, digits, after) = split_digits(&line);
                    let x = draw(Style::Caption, p.ink_muted, *x, *top, before);
                    let x = draw(Style::CaptionStrong, p.ink, x, *top, digits);
                    draw(Style::Caption, p.ink_muted, x, *top, after);
                }
                Item::Button(part, r) => {
                    let Some(label) = part.label() else { continue };
                    let pressed = s.pointer.target(*part) == PRESSED;
                    let drop = if pressed { px(PRESS_DROP, dpi) } else { 0 };
                    let wide: Vec<u16> = label.encode_utf16().collect();
                    let old = SelectObject(dc, fonts.get(Style::BodyStrong).into());
                    let mut size = SIZE::default();
                    let _ = GetTextExtentPoint32W(dc, &wide, &mut size);
                    SelectObject(dc, old);
                    let line = px(Style::BodyStrong.line_height(), dpi);
                    let x = r.left + (r.right - r.left - size.cx) / 2;
                    let top = r.top + (r.bottom - r.top - line) / 2 + drop;
                    draw(Style::BodyStrong, p.ink, x, top, label);
                }
                _ => {}
            }
        }
    }
}

// ---- Window procedure ----

fn point(lparam: LPARAM) -> (i32, i32) {
    let x = (lparam.0 & 0xffff) as u16 as i16;
    let y = ((lparam.0 >> 16) & 0xffff) as u16 as i16;
    (x.into(), y.into())
}

/// Repaints and runs frames until the fills settle.
unsafe fn refresh(hwnd: HWND) {
    // SAFETY: per the caller, `hwnd` is the live popup window.
    unsafe {
        let _ = InvalidateRect(Some(hwnd), None, false);
        with_state(|s| s.last_frame = Instant::now());
        start_frames(hwnd);
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: Win32 calls on the live popup window; no state borrow is held across a call that
    // sends this window a message.
    unsafe {
        match msg {
            // Clicks land without activating the window, so focus stays put.
            WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
            WM_ERASEBKGND => LRESULT(1),
            WM_PAINT => {
                paint(hwnd);
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                let (x, y) = point(lparam);
                let down = wparam.0 & MK_LBUTTON.0 as usize != 0;
                let track = with_state(|s| {
                    let hot = if s.fade_to > 0.0 {
                        s.layout.hit(x, y)
                    } else {
                        None
                    };
                    let changed = s.pointer.moved(hot, down);
                    let track = !s.tracking;
                    s.tracking = true;
                    (changed, track)
                });
                if let Some((changed, track)) = track {
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
                with_state(|s| {
                    s.tracking = false;
                    s.pointer.left(down);
                });
                refresh(hwnd);
                LRESULT(0)
            }
            WM_LBUTTONDOWN => {
                let (x, y) = point(lparam);
                let hit = with_state(|s| {
                    let hit = (s.fade_to > 0.0).then(|| s.layout.hit(x, y)).flatten();
                    s.pointer.down(hit);
                    hit
                })
                .flatten();
                if hit.is_some() {
                    SetCapture(hwnd);
                    refresh(hwnd);
                }
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                let (x, y) = point(lparam);
                let done = with_state(|s| {
                    let over = s.layout.hit(x, y);
                    s.pointer.up(over).map(|p| (p, s.owner))
                })
                .flatten();
                let _ = ReleaseCapture();
                refresh(hwnd);
                match done {
                    // × hides the popup; the occurrence stays pending (SPEC §6).
                    Some((Part::Close, _)) => hide(hwnd),
                    Some((Part::Snooze, owner)) => {
                        let _ = PostMessageW(Some(owner), WM_POPUP_SNOOZE, WPARAM(0), LPARAM(0));
                    }
                    Some((Part::Skip, owner)) => {
                        let _ = PostMessageW(Some(owner), WM_POPUP_SKIP, WPARAM(0), LPARAM(0));
                    }
                    None => {}
                }
                LRESULT(0)
            }
            WM_CAPTURECHANGED => {
                with_state(|s| s.pointer.cancel());
                refresh(hwnd);
                LRESULT(0)
            }
            WM_CLOSE => {
                hide(hwnd);
                LRESULT(0)
            }
            WM_TIMER if wparam.0 == WATER_TIMER => {
                hide(hwnd);
                LRESULT(0)
            }
            WM_TIMER if wparam.0 == FRAME_TIMER => {
                frame(hwnd);
                LRESULT(0)
            }
            WM_SETTINGCHANGE => {
                if let Some(theme) = theme::theme_on_setting_change(lparam) {
                    with_state(|s| s.theme = theme);
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            WM_DPICHANGED => {
                let dpi = (wparam.0 & 0xffff) as u32;
                if let Some(place) = with_state(|s| s.relayout(dpi)) {
                    move_to(hwnd, place);
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every character 7px wide, at any style.
    struct Fixed;
    impl Measure for Fixed {
        fn width(&self, _: Style, text: &str) -> i32 {
            7 * text.chars().count() as i32
        }
    }

    fn prompt(water: bool, snooze: bool) -> Content {
        Content::Prompt {
            nudge: Nudge::Stand,
            title: Nudge::Stand.title(),
            line: Nudge::Stand.line(2),
            water,
            snooze,
        }
    }

    fn buttons(l: &Layout) -> Vec<(Part, RECT)> {
        l.items
            .iter()
            .filter_map(|i| match i {
                Item::Button(p, r) => Some((*p, *r)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn wrap_breaks_at_spaces_within_the_width() {
        let w = |t: &str| t.len() as i32;
        assert_eq!(wrap("aa bb cc", 5, w), ["aa bb", "cc"]);
        assert_eq!(wrap("aa bb cc", 100, w), ["aa bb cc"]);
        assert_eq!(wrap("toolongword x", 4, w), ["toolongword", "x"]);
        assert!(wrap("", 4, w).is_empty());
    }

    #[test]
    fn prompt_layout_follows_the_design_at_100_percent() {
        let l = layout(&prompt(false, true), 96, &Fixed);
        assert_eq!(l.width, SIZE_POPUP);
        // Header: dot 8 centred in 18 at the padding.
        assert!(l.items.contains(&Item::Dot(rect(16, 21, 8, 8))));
        // Close ×: 24 square, 4 past the right padding edge, centred on the header.
        assert_eq!(
            buttons(&l)[0],
            (Part::Close, rect(360 - 16 + 4 - 24, 13, 24, 24))
        );
        // Glyph 12 under the header, title text 8 right of it.
        assert!(
            l.items
                .contains(&Item::Glyph(Glyph::Stand, rect(16, 50, 20, 20)))
        );
        let title = l.items.iter().find_map(|i| match i {
            Item::Text {
                style: Style::Title,
                x,
                top,
                ..
            } => Some((*x, *top)),
            _ => None,
        });
        assert_eq!(title, Some((44, 46)));
        // The line, aligned with the title text, 4 under the title row, wrapped to two lines of
        // 20 (55 chars × 7 = 385 > 300).
        let lines: Vec<_> = l
            .items
            .iter()
            .filter_map(|i| match i {
                Item::Text {
                    style: Style::Body,
                    ink: Ink::Muted,
                    x,
                    top,
                    ..
                } => Some((*x, *top)),
                _ => None,
            })
            .collect();
        assert_eq!(lines, [(44, 78), (44, 98)]);
        // Bar 16 under the line, full width inside the padding, 4 tall; countdown 6 under it.
        assert!(l.items.contains(&Item::Bar(rect(16, 134, 328, 4))));
        assert!(l.items.contains(&Item::Countdown { x: 16, top: 144 }));
        // Buttons 16 under the countdown, right-aligned, 8 apart, Skip last.
        let b = buttons(&l);
        let snooze_w = 7 * 12 + 2 * (12 + 1);
        let skip_w = 7 * 4 + 2 * (12 + 1);
        assert_eq!(
            b[1],
            (
                Part::Snooze,
                rect(344 - skip_w - 8 - snooze_w, 178, snooze_w, 32)
            )
        );
        assert_eq!(b[2], (Part::Skip, rect(344 - skip_w, 178, skip_w, 32)));
        assert_eq!(l.height, 178 + 32 + 16);
    }

    #[test]
    fn riding_water_adds_its_row_and_after_the_snooze_skip_stands_alone() {
        let plain = layout(&prompt(false, false), 96, &Fixed);
        let water = layout(&prompt(true, false), 96, &Fixed);
        assert_eq!(water.height, plain.height + 8 + 20);
        assert!(
            water
                .items
                .contains(&Item::Glyph(Glyph::Water, rect(44, 128, 16, 16)))
        );
        assert_eq!(
            buttons(&plain).iter().map(|b| b.0).collect::<Vec<_>>(),
            [Part::Close, Part::Skip]
        );
    }

    #[test]
    fn water_alone_is_one_row_without_header_or_buttons() {
        let l = layout(&Content::WaterAlone, 96, &Fixed);
        assert_eq!(l.height, 12 + 20 + 12);
        assert!(buttons(&l).is_empty());
        assert!(!l.items.iter().any(|i| matches!(i, Item::Dot(_))));
        assert!(
            l.items
                .contains(&Item::Glyph(Glyph::Water, rect(16, 12, 20, 20)))
        );
    }

    #[test]
    fn notice_has_the_header_and_no_buttons_but_the_close() {
        let l = layout(&Content::Notice, 96, &Fixed);
        assert_eq!(
            buttons(&l).iter().map(|b| b.0).collect::<Vec<_>>(),
            [Part::Close]
        );
    }

    #[test]
    fn layout_scales_with_the_monitor() {
        for dpi in [144, 192] {
            let l = layout(&prompt(true, true), dpi, &Fixed);
            assert_eq!(l.width, px(SIZE_POPUP, dpi));
            assert!(
                l.items.contains(&Item::Bar(rect(
                    px(16, dpi),
                    l.items
                        .iter()
                        .find_map(|i| match i {
                            Item::Bar(r) => Some(r.top),
                            _ => None,
                        })
                        .unwrap(),
                    px(328, dpi),
                    px(4, dpi)
                )))
            );
            assert_eq!(buttons(&l)[2].1.bottom, l.height - px(16, dpi));
        }
    }

    #[test]
    fn hit_finds_the_part_under_the_pointer() {
        let l = layout(&prompt(false, true), 96, &Fixed);
        let skip = buttons(&l)[2].1;
        assert_eq!(l.hit(skip.left, skip.top), Some(Part::Skip));
        assert_eq!(l.hit(skip.right, skip.top), None);
        assert_eq!(l.hit(330, 20), Some(Part::Close));
        assert_eq!(l.hit(100, 100), None);
    }

    #[test]
    fn a_press_acts_only_when_released_on_the_part_it_started_on() {
        let mut p = Pointer::default();
        p.down(Some(Part::Snooze));
        assert_eq!(p.target(Part::Snooze), PRESSED);
        assert_eq!(p.up(Some(Part::Snooze)), Some(Part::Snooze));
        p.down(Some(Part::Snooze));
        assert_eq!(p.up(Some(Part::Skip)), None);
        assert_eq!(p.target(Part::Skip), HOVER);
        // Pressed outside every part: no part is pressed, so no release acts.
        p.down(None);
        assert_eq!(p.up(Some(Part::Skip)), None);
    }

    #[test]
    fn a_press_released_outside_the_popup_never_acts_later() {
        // Released outside: the up went to another window. The next move without the button
        // ends the press.
        let mut p = Pointer::default();
        p.down(Some(Part::Snooze));
        p.moved(None, true);
        assert!(p.moved(Some(Part::Snooze), false));
        assert_eq!(p.target(Part::Snooze), HOVER);
        assert_eq!(p.up(Some(Part::Snooze)), None);

        // Or the leave, when the button is up by then.
        p.down(Some(Part::Snooze));
        p.left(false);
        assert_eq!(p.up(Some(Part::Snooze)), None);

        // Leaving with the button still down keeps the press, as in a native button.
        p.down(Some(Part::Snooze));
        p.left(true);
        p.moved(Some(Part::Snooze), true);
        assert_eq!(p.up(Some(Part::Snooze)), Some(Part::Snooze));
    }

    #[test]
    fn countdown_digits_are_split_out() {
        let line = countdown_text(Duration::from_secs(119));
        assert_eq!(
            split_digits(&line),
            ("Away ", "1:59", " more and it counts")
        );
        assert_eq!(split_digits("none"), ("none", "", ""));
    }

    #[test]
    fn bar_fills_with_away_time() {
        let five = Duration::from_secs(300);
        assert_eq!(bar_fraction(five, five), 0.0);
        assert_eq!(bar_fraction(Duration::ZERO, five), 1.0);
        assert_eq!(bar_fraction(Duration::from_secs(150), five), 0.5);
    }
}
