//! The design tokens as Win32 values (DESIGN.md §9).
//!
//! Every window takes its colours, sizes, fonts and timings from here. No window procedure holds
//! a colour or size literal. Sizes are at 96 DPI; [`px`] scales them for a monitor. The popup
//! and the form kit's windows follow the Windows app theme ([`app_theme`]), and the kit's
//! controls take their colours by role from [`Colours`], which also covers high contrast. The
//! native-control dialogs stay [`Theme::Light`] until they move to the kit.

use std::time::Duration;

use windows::Win32::Foundation::{COLORREF, ERROR_SUCCESS, LPARAM};
use windows::Win32::Graphics::Gdi::{
    CLEARTYPE_QUALITY, COLOR_BTNFACE, COLOR_GRAYTEXT, COLOR_HIGHLIGHT, COLOR_HIGHLIGHTTEXT,
    COLOR_WINDOW, COLOR_WINDOWTEXT, CreateFontIndirectW, DEFAULT_CHARSET, DeleteObject,
    EnumFontFamiliesExW, GetDC, GetSysColor, HFONT, LOGFONTW, ReleaseDC, SYS_COLOR_INDEX,
    TEXTMETRICW,
};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::Win32::UI::Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW};
use windows::Win32::UI::WindowsAndMessaging::{
    SPI_GETCLIENTAREAANIMATION, SPI_GETHIGHCONTRAST, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
    SystemParametersInfoW,
};
use windows::core::{BOOL, PCWSTR, w};

// ---- Colour (DESIGN.md §3) ----

/// A `COLORREF` from a `0xRRGGBB` value as written in DESIGN.md. Win32 stores it as `0x00BBGGRR`.
pub const fn rgb(hex: u32) -> COLORREF {
    let (r, g, b) = ((hex >> 16) & 0xff, (hex >> 8) & 0xff, hex & 0xff);
    COLORREF(b << 16 | g << 8 | r)
}

/// The Windows app theme.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Theme {
    Light,
    Dark,
}

impl Theme {
    pub const fn palette(self) -> &'static Palette {
        match self {
            Theme::Light => &LIGHT,
            Theme::Dark => &DARK,
        }
    }
}

/// One colour per token. See DESIGN.md §3 for where each one goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    /// Ground of the wizard and the settings window.
    pub surface: COLORREF,
    /// Ground of the popup.
    pub surface_raised: COLORREF,
    /// Dialog footer band, countdown bar track, hover fill of secondary and ghost buttons.
    pub surface_sunken: COLORREF,
    /// All main text.
    pub ink: COLORREF,
    /// The nudge line, the countdown, captions and notes.
    pub ink_muted: COLORREF,
    /// Hairlines, press fill of secondary and ghost buttons.
    pub line: COLORREF,
    /// Control borders.
    pub line_strong: COLORREF,
    /// The breakpoint dot. Never text.
    pub dot: COLORREF,
    /// Fill of the one primary button in a dialog.
    pub action: COLORREF,
    /// Primary button fill on hover and press.
    pub action_hover: COLORREF,
    /// Label on `action` and `action_hover`.
    pub on_action: COLORREF,
    /// Stand/Move glyph and bar.
    pub stand: COLORREF,
    /// Water glyph.
    pub water: COLORREF,
    /// Pause glyph and bar.
    pub pause: COLORREF,
    /// The hollow tray ring while quiet or Off.
    pub quiet: COLORREF,
    /// Keyboard focus ring in dialogs.
    pub focus: COLORREF,
}

pub const LIGHT: Palette = Palette {
    surface: rgb(0xfafaf9),
    surface_raised: rgb(0xffffff),
    surface_sunken: rgb(0xf1f0ee),
    ink: rgb(0x1c1b1a),
    ink_muted: rgb(0x57534f),
    line: rgb(0xe3e1de),
    line_strong: rgb(0x8c8782),
    dot: rgb(0xe5484d),
    action: rgb(0xc8333a),
    action_hover: rgb(0xb02a31),
    on_action: rgb(0xffffff),
    stand: rgb(0xb5651d),
    water: rgb(0x2b6cb0),
    pause: rgb(0x2f7d5b),
    quiet: rgb(0x969696),
    focus: rgb(0x1c1b1a),
};

pub const DARK: Palette = Palette {
    surface: rgb(0x1c1b1a),
    surface_raised: rgb(0x262422),
    surface_sunken: rgb(0x141312),
    ink: rgb(0xf2f1ef),
    ink_muted: rgb(0xb3aea8),
    line: rgb(0x3a3835),
    line_strong: rgb(0x77726d),
    dot: rgb(0xe5484d),
    action: rgb(0xc8333a),
    action_hover: rgb(0xb02a31),
    on_action: rgb(0xffffff),
    stand: rgb(0xf0a04b),
    water: rgb(0x6aaef0),
    pause: rgb(0x5fc495),
    quiet: rgb(0x969696),
    focus: rgb(0xf2f1ef),
};

// ---- Spacing, shape and size (DESIGN.md §5 and §6), at 96 DPI ----

/// Scales `value` from 96 DPI to `dpi`, rounding half away from zero like `MulDiv`.
pub const fn px(value: i32, dpi: u32) -> i32 {
    let n = value as i64 * dpi as i64;
    let half = if n < 0 { -48 } else { 48 };
    ((n + half) / 96) as i32
}

pub const SPACE_1: i32 = 4;
pub const SPACE_2: i32 = 8;
pub const SPACE_3: i32 = 12;
pub const SPACE_4: i32 = 16;
pub const SPACE_6: i32 = 24;
pub const SPACE_8: i32 = 32;

/// Buttons and the profile options.
pub const RADIUS_SM: i32 = 4;
/// Cards, selectable cards and the drop-down menu. The popup and the menu window get theirs from
/// DWM (`DWMWCP_ROUND`); painted cards use this value. `radius-full` is half a shape's height,
/// so it needs no const.
pub const RADIUS_MD: i32 = 8;

/// Popup width. Height follows the content.
pub const SIZE_POPUP: i32 = 360;
/// Button height.
pub const SIZE_BUTTON: i32 = 32;
/// Nudge glyph beside the popup title.
pub const SIZE_GLYPH: i32 = 20;
/// Glyph beside the riding Water line.
pub const SIZE_GLYPH_INLINE: i32 = 16;
/// The dot in the popup header.
pub const SIZE_MARK: i32 = 8;
/// Countdown bar height.
pub const SIZE_BAR: i32 = 4;

/// Popup header row height.
pub const POPUP_HEADER: i32 = 18;
/// Hit area of the popup's close ×.
pub const CLOSE_HIT: i32 = 24;
/// Gap from the countdown bar to the countdown text.
pub const BAR_GAP: i32 = 6;
/// Least width of a primary button.
pub const PRIMARY_MIN_WIDTH: i32 = 80;
/// Width of the focus ring and its gap to the control.
pub const FOCUS_RING: i32 = 2;
/// Water alone padding, top and bottom.
pub const WATER_ALONE_PAD_Y: i32 = 12;
/// Water alone padding, left and right.
pub const WATER_ALONE_PAD_X: i32 = 16;
/// A hairline: the secondary button border and the Windows 10 popup outline.
pub const HAIRLINE: i32 = 1;
/// How far a pressed button's label moves down.
pub const PRESS_DROP: i32 = 1;
/// How far the popup's close × hit area reaches past the popup padding, right, top and bottom,
/// so the × itself lines up with the padding edge.
pub const CLOSE_OVERHANG: i32 = 4;
/// The close × mark's box, centred in its hit area. The × is two strokes from corner to corner,
/// [`CLOSE_INSET`] in from each, [`CLOSE_STROKE`] wide; all three scale with the DPI.
pub const CLOSE_MARK: f32 = 10.0;
pub const CLOSE_INSET: f32 = 1.0;
pub const CLOSE_STROKE: f32 = 1.5;

// ---- Painted dialog controls (the form kit, issue #77), at 96 DPI ----

/// Height of the stepper, the drop-down and the segmented track. Same as a button.
pub const SIZE_FIELD: i32 = SIZE_BUTTON;

/// Switch track.
pub const SWITCH_W: i32 = 40;
pub const SWITCH_H: i32 = 22;
/// Switch knob at rest, and while the pointer is over the switch.
pub const KNOB: i32 = 14;
pub const KNOB_HOT: i32 = 16;
/// Room between the resting knob and the track edge.
pub const KNOB_INSET: i32 = 3;

/// The checkbox square and the selectable card's radio mark.
pub const CHECKBOX: i32 = 18;
pub const RADIO: i32 = 18;
/// The dot inside a selected radio mark.
pub const RADIO_DOT: i32 = 8;
/// Border of the checkbox and the radio mark, scaled with the DPI but not rounded.
pub const BORDER_THICK: f32 = 1.5;

/// Day pill (one work day).
pub const PILL_W: i32 = 52;
pub const PILL_H: i32 = 32;

/// Stepper: the − and + zones, the value between them, and the marks in the zones.
pub const STEPPER_ZONE: i32 = 28;
pub const STEPPER_VALUE: i32 = 56;
pub const STEPPER_MARK: i32 = 12;
/// Stroke of the − and + marks, the check mark and the chevron, scaled but not rounded.
pub const MARK_STROKE: f32 = 1.5;
/// How many steps Page Up and Page Down take.
pub const PAGE_STEPS: i32 = 5;
/// The text caret in a stepper being typed in.
pub const CARET: i32 = 1;

/// Segmented toggle: the track's inner padding, the gap between segments, and each segment.
pub const SEGMENT_PAD: i32 = 2;
pub const SEGMENT_GAP: i32 = 2;
pub const SEGMENT_H: i32 = 28;
pub const SEGMENT_MIN_W: i32 = 50;

/// Drop-down chevron, and its gap to the field's right edge.
pub const CHEVRON: i32 = 12;
pub const CHEVRON_RIGHT: i32 = 10;

/// Drop-down menu: padding round the items, each item's padding, the check mark at the left of
/// the current item, and the gap between the field and the menu.
pub const MENU_PAD: i32 = 4;
pub const MENU_ITEM_PAD_Y: i32 = 8;
pub const MENU_ITEM_PAD_X: i32 = 10;
pub const MENU_CHECK: i32 = 14;
pub const MENU_GAP: i32 = 4;

/// Selectable card padding, top and bottom, then left and right.
pub const CHOICE_PAD_Y: i32 = 14;
pub const CHOICE_PAD_X: i32 = 16;

/// The scrollbar of a dialog taller than its work area (issue #84): the thumb's width at rest
/// and under the pointer, its gap to the window's right edge and to the footer band, and its
/// shortest length. The thumb is `line-strong` with round ends, on no track.
pub const SCROLLBAR_W: i32 = 4;
pub const SCROLLBAR_HOT_W: i32 = 8;
pub const SCROLLBAR_INSET: i32 = 2;
pub const SCROLLBAR_MIN: i32 = 32;
/// How far one wheel line scrolls a dialog: one `body` line.
pub const SCROLL_LINE: i32 = Style::Body.line_height();
/// Room kept round a control that keyboard focus scrolls into view.
pub const SCROLL_ROOM: i32 = SPACE_4;

// ---- The tray flyout (issue #89), at 96 DPI ----

/// Flyout width. Height follows the content.
pub const SIZE_FLYOUT: i32 = 320;
/// The status dot in the flyout header, and the width of its ring while quiet or Off.
pub const STATUS_DOT: i32 = 10;
pub const STATUS_RING: f32 = 2.0;
/// The icon on a footer button (gear, power), and its gap to the label.
pub const SIZE_ICON: i32 = 16;
/// Stroke of the power icon, scaled but not rounded.
pub const ICON_STROKE: f32 = 1.5;

/// The check mark, as points on its square from 0 to 1, drawn with [`MARK_STROKE`].
pub const CHECK_POINTS: [(f32, f32); 3] = [(0.22, 0.52), (0.42, 0.72), (0.78, 0.3)];
/// The drop-down chevron, as points on its square from 0 to 1.
pub const CHEVRON_POINTS: [(f32, f32); 3] = [(0.2, 0.36), (0.5, 0.66), (0.8, 0.36)];

// ---- Motion (DESIGN.md §8) ----

pub const FADE_IN: Duration = Duration::from_millis(150);
pub const FADE_OUT: Duration = Duration::from_millis(100);
pub const HOVER_IN: Duration = Duration::from_millis(150);
pub const HOVER_OUT: Duration = Duration::from_millis(120);
pub const PRESS: Duration = Duration::from_millis(90);
/// A switch knob sliding between off and on.
pub const KNOB_SLIDE: Duration = Duration::from_millis(150);

/// Whether Windows animation is on. When it is off, everything changes at once.
pub fn animations_on() -> bool {
    let mut on = BOOL(1);
    // SAFETY: `on` is a valid BOOL out-pointer for this action.
    let read = unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            Some(&mut on as *mut BOOL as _),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    read.is_err() || on.as_bool()
}

// ---- App theme ----

const PERSONALIZE: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");

/// The Windows app theme now. Light when the value is missing, as on a fresh Windows.
pub fn app_theme() -> Theme {
    let mut light = 1u32;
    let mut size = size_of::<u32>() as u32;
    // SAFETY: both names are static wide strings; `light` holds the `size` bytes asked for.
    let read = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            PERSONALIZE,
            w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut light as *mut u32 as _),
            Some(&mut size),
        )
    };
    if read == ERROR_SUCCESS && light == 0 {
        Theme::Dark
    } else {
        Theme::Light
    }
}

/// For `WM_SETTINGCHANGE` in a top-level window: the app theme read again when the change is
/// `ImmersiveColorSet` (the user switched light or dark), else `None`.
pub fn theme_on_setting_change(lparam: LPARAM) -> Option<Theme> {
    let area = PCWSTR(lparam.0 as *const u16);
    // SAFETY: for WM_SETTINGCHANGE, a non-null lParam is a NUL-terminated wide string.
    let is_colour_set = !area.is_null()
        && unsafe { area.as_wide() }
            .iter()
            .copied()
            .eq("ImmersiveColorSet".encode_utf16());
    is_colour_set.then(app_theme)
}

// ---- Control colours (the form kit) ----

/// The colours a painted dialog control uses: the palette's tokens by role, or, in high
/// contrast, the system colours.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Colours {
    /// Window ground (`surface`).
    pub ground: COLORREF,
    /// Ground of cards and controls (`surface-raised`).
    pub raised: COLORREF,
    /// Footer band and the segmented track (`surface-sunken`).
    pub sunken: COLORREF,
    /// Main text and marks (`ink`).
    pub ink: COLORREF,
    /// Notes, units at rest, the chevron (`ink-muted`).
    pub muted: COLORREF,
    /// Hairlines and card outlines (`line`).
    pub hairline: COLORREF,
    /// Control edges (`line-strong`).
    pub edge: COLORREF,
    /// Fill of a control that is on or selected (`ink`).
    pub on: COLORREF,
    /// Label and knob on [`Colours::on`] (`surface`).
    pub on_ink: COLORREF,
    /// Hover fill (`surface-sunken`).
    pub hover: COLORREF,
    /// Press fill (`line`).
    pub press: COLORREF,
    /// Labels and marks on the hover and press fills (`ink`).
    pub hover_ink: COLORREF,
    /// Edge of a switch, checkbox or radio mark under the pointer (`ink`).
    pub hover_edge: COLORREF,
    /// The primary button (`action`, `action-hover`, `on-action`).
    pub action: COLORREF,
    pub action_hover: COLORREF,
    pub on_action: COLORREF,
    /// The keyboard focus ring (`focus`).
    pub focus: COLORREF,
    /// The selected segment of a segmented toggle: fill, label and edge (`surface-raised`,
    /// `ink`, `line`).
    pub chosen: COLORREF,
    pub on_chosen: COLORREF,
    pub chosen_edge: COLORREF,
    /// The highlighted row of an open menu and its text (`surface-sunken`, `ink`).
    pub hot_row: COLORREF,
    pub on_hot_row: COLORREF,
    /// The nudge glyphs.
    pub stand: COLORREF,
    pub water: COLORREF,
    pub pause: COLORREF,
    /// The flyout's status dot while on (`dot`) and its ring while quiet or Off (`quiet`).
    pub dot: COLORREF,
    pub quiet: COLORREF,
    /// Whether menus may cast a shadow. Never in high contrast.
    pub shadows: bool,
}

impl Colours {
    /// The tokens of `theme`.
    pub const fn of(theme: Theme) -> Colours {
        let p = theme.palette();
        Colours {
            ground: p.surface,
            raised: p.surface_raised,
            sunken: p.surface_sunken,
            ink: p.ink,
            muted: p.ink_muted,
            hairline: p.line,
            edge: p.line_strong,
            on: p.ink,
            on_ink: p.surface,
            hover: p.surface_sunken,
            press: p.line,
            hover_ink: p.ink,
            hover_edge: p.ink,
            action: p.action,
            action_hover: p.action_hover,
            on_action: p.on_action,
            focus: p.focus,
            chosen: p.surface_raised,
            on_chosen: p.ink,
            chosen_edge: p.line,
            hot_row: p.surface_sunken,
            on_hot_row: p.ink,
            stand: p.stand,
            water: p.water,
            pause: p.pause,
            dot: p.dot,
            quiet: p.quiet,
            shadows: true,
        }
    }

    /// High contrast: every colour from `system`, which reads a system colour. Text and edges in
    /// `COLOR_WINDOWTEXT` on `COLOR_WINDOW`, the segmented track and footer band in
    /// `COLOR_BTNFACE`, hairlines in `COLOR_GRAYTEXT`, and what is under the pointer, on,
    /// selected, highlighted or primary in `COLOR_HIGHLIGHT` with `COLOR_HIGHLIGHTTEXT`.
    pub fn high_contrast(system: impl Fn(SYS_COLOR_INDEX) -> COLORREF) -> Colours {
        let (window, text) = (system(COLOR_WINDOW), system(COLOR_WINDOWTEXT));
        let (highlight, on_highlight) = (system(COLOR_HIGHLIGHT), system(COLOR_HIGHLIGHTTEXT));
        Colours {
            ground: window,
            raised: window,
            sunken: system(COLOR_BTNFACE),
            ink: text,
            muted: text,
            hairline: system(COLOR_GRAYTEXT),
            edge: text,
            on: highlight,
            on_ink: on_highlight,
            hover: highlight,
            press: highlight,
            hover_ink: on_highlight,
            hover_edge: highlight,
            action: highlight,
            action_hover: highlight,
            on_action: on_highlight,
            focus: text,
            chosen: highlight,
            on_chosen: on_highlight,
            chosen_edge: highlight,
            hot_row: highlight,
            on_hot_row: on_highlight,
            stand: text,
            water: text,
            pause: text,
            dot: text,
            quiet: text,
            shadows: false,
        }
    }

    /// The colours to paint with now: the system's in high contrast, else `theme`'s.
    pub fn now(theme: Theme) -> Colours {
        if high_contrast() {
            // SAFETY: GetSysColor takes any index and has no pointers.
            Colours::high_contrast(|i| COLORREF(unsafe { GetSysColor(i) }))
        } else {
            Colours::of(theme)
        }
    }
}

/// Whether Windows high contrast is on.
pub fn high_contrast() -> bool {
    let mut hc = HIGHCONTRASTW {
        cbSize: size_of::<HIGHCONTRASTW>() as u32,
        ..Default::default()
    };
    // SAFETY: `hc` is a HIGHCONTRASTW with its size set, as this action wants.
    let read = unsafe {
        SystemParametersInfoW(
            SPI_GETHIGHCONTRAST,
            hc.cbSize,
            Some(&mut hc as *mut HIGHCONTRASTW as _),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    read.is_ok() && hc.dwFlags.contains(HCF_HIGHCONTRASTON)
}

/// The High Contrast Black system colours, for pictures of the high-contrast look on a machine
/// that is not in high contrast.
#[cfg(test)]
pub fn high_contrast_black(index: SYS_COLOR_INDEX) -> COLORREF {
    match index {
        COLOR_WINDOW | COLOR_BTNFACE | COLOR_HIGHLIGHTTEXT => rgb(0x000000),
        COLOR_WINDOWTEXT => rgb(0xffffff),
        COLOR_HIGHLIGHT => rgb(0x1aebff),
        COLOR_GRAYTEXT => rgb(0x3ff23f),
        _ => rgb(0xffffff),
    }
}

// ---- Type (DESIGN.md §4) ----

/// The type styles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    TitleLg,
    Title,
    Body,
    BodyStrong,
    Caption,
    CaptionStrong,
}

impl Style {
    pub const ALL: [Style; 6] = [
        Style::TitleLg,
        Style::Title,
        Style::Body,
        Style::BodyStrong,
        Style::Caption,
        Style::CaptionStrong,
    ];

    /// Font size in px at 96 DPI.
    pub const fn size(self) -> i32 {
        match self {
            Style::TitleLg => 28,
            Style::Title => 20,
            Style::Body | Style::BodyStrong => 14,
            Style::Caption | Style::CaptionStrong => 13,
        }
    }

    /// Line height in px at 96 DPI.
    pub const fn line_height(self) -> i32 {
        match self {
            Style::TitleLg => 36,
            Style::Title => 28,
            Style::Body | Style::BodyStrong => 20,
            Style::Caption | Style::CaptionStrong => 18,
        }
    }

    pub const fn weight(self) -> i32 {
        match self {
            Style::TitleLg | Style::Title | Style::BodyStrong | Style::CaptionStrong => 600,
            Style::Body | Style::Caption => 400,
        }
    }

    const fn display(self) -> bool {
        matches!(self, Style::TitleLg | Style::Title)
    }

    /// The GDI face for this style: Segoe UI Variable when `variable`, else Segoe UI.
    ///
    /// GDI lists each semibold as a family of its own, under a name cut to 31 characters. Asked
    /// for weight 600 in the regular family, it fakes a bold from the regular face instead.
    pub const fn face(self, variable: bool) -> PCWSTR {
        let semibold = self.weight() == 600;
        match (variable, self.display(), semibold) {
            (true, true, true) => w!("Segoe UI Variable Display Semib"),
            (true, true, false) => w!("Segoe UI Variable Display"),
            (true, false, true) => w!("Segoe UI Variable Text Semibold"),
            (true, false, false) => w!("Segoe UI Variable Text"),
            (false, _, true) => w!("Segoe UI Semibold"),
            (false, _, false) => w!("Segoe UI"),
        }
    }
}

thread_local! {
    static VARIABLE: bool = Style::ALL.iter().all(|s| has_face(s.face(true)));
}

/// Whether every Segoe UI Variable face is installed (Windows 11). Read once per thread.
pub fn has_variable() -> bool {
    VARIABLE.with(|v| *v)
}

/// Whether a font family named `face` is installed.
fn has_face(face: PCWSTR) -> bool {
    unsafe extern "system" fn found(
        _: *const LOGFONTW,
        _: *const TEXTMETRICW,
        _: u32,
        hit: LPARAM,
    ) -> i32 {
        // SAFETY: `hit` is the `bool` passed below, alive for the whole enumeration.
        unsafe { *(hit.0 as *mut bool) = true };
        0
    }
    let mut query = LOGFONTW {
        lfCharSet: DEFAULT_CHARSET,
        ..Default::default()
    };
    set_face(&mut query, face);
    let mut hit = false;
    // SAFETY: the screen DC is released before return; `hit` outlives the call.
    unsafe {
        let dc = GetDC(None);
        EnumFontFamiliesExW(
            dc,
            &query,
            Some(found),
            LPARAM(&mut hit as *mut bool as isize),
            0,
        );
        ReleaseDC(None, dc);
    }
    hit
}

fn set_face(font: &mut LOGFONTW, face: PCWSTR) {
    // SAFETY: every face passed here is a static NUL-terminated wide string.
    let name = unsafe { face.as_wide() };
    let n = name.len().min(font.lfFaceName.len() - 1);
    font.lfFaceName[..n].copy_from_slice(&name[..n]);
    font.lfFaceName[n] = 0;
}

/// The font description for `style` at `dpi`, in Segoe UI Variable when `variable`.
fn logfont(style: Style, dpi: u32, variable: bool) -> LOGFONTW {
    let mut font = LOGFONTW {
        // Negative: the em height in pixels, so 14 means 14px type, as in the design.
        lfHeight: -px(style.size(), dpi),
        lfWeight: style.weight(),
        lfCharSet: DEFAULT_CHARSET,
        lfQuality: CLEARTYPE_QUALITY,
        ..Default::default()
    };
    set_face(&mut font, style.face(variable));
    font
}

/// One font per style for one DPI. Freed on drop.
pub struct Fonts {
    dpi: u32,
    fonts: [HFONT; Style::ALL.len()],
}

impl Fonts {
    pub fn new(dpi: u32) -> Fonts {
        let variable = has_variable();
        // SAFETY: each LOGFONTW is a complete local value.
        let fonts = Style::ALL.map(|s| unsafe { CreateFontIndirectW(&logfont(s, dpi, variable)) });
        Fonts { dpi, fonts }
    }

    pub fn get(&self, style: Style) -> HFONT {
        self.fonts[style as usize]
    }
}

impl Drop for Fonts {
    fn drop(&mut self) {
        for font in self.fonts {
            if !font.is_invalid() {
                // SAFETY: this set created the font and nothing else frees it.
                let _ = unsafe { DeleteObject(font.into()) };
            }
        }
    }
}

/// The fonts for one window, cached for its current DPI.
///
/// When the DPI changes, the old set is kept until the next change, so controls can still use
/// it until they get `WM_SETFONT` with the new one. Nothing is freed while in use.
#[derive(Default)]
pub struct FontCache {
    current: Option<Fonts>,
    retired: Option<Fonts>,
}

impl FontCache {
    pub const fn new() -> FontCache {
        FontCache {
            current: None,
            retired: None,
        }
    }

    /// The fonts at `dpi`, built on first use and again when the DPI changes.
    pub fn at(&mut self, dpi: u32) -> &Fonts {
        if self.current.as_ref().is_none_or(|f| f.dpi != dpi) {
            self.retired = self.current.replace(Fonts::new(dpi));
        }
        self.current.as_ref().expect("built above")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Graphics::Gdi::{GetTextFaceW, GetTextMetricsW, SelectObject};

    #[test]
    fn rgb_swaps_to_bgr() {
        assert_eq!(rgb(0x123456), COLORREF(0x563412));
        assert_eq!(LIGHT.ink_muted, COLORREF(0x4f5357));
    }

    #[test]
    fn px_rounds_the_same_way_at_every_scale() {
        // (value at 96 DPI, at 100 %, 125 %, 150 %, 200 %)
        let cases = [
            (1, [1, 1, 2, 2]),
            (2, [2, 3, 3, 4]),
            (4, [4, 5, 6, 8]),
            (6, [6, 8, 9, 12]),
            (13, [13, 16, 20, 26]),
            (14, [14, 18, 21, 28]),
            (18, [18, 23, 27, 36]),
            (360, [360, 450, 540, 720]),
            (-2, [-2, -3, -3, -4]),
        ];
        for (value, want) in cases {
            let got = [96, 120, 144, 192].map(|dpi| px(value, dpi));
            assert_eq!(got, want, "px({value})");
        }
        for dpi in [96, 120, 144, 192] {
            for value in 0..=400 {
                let exact = value as f64 * dpi as f64 / 96.0;
                assert_eq!(px(value, dpi), exact.round() as i32, "px({value}, {dpi})");
            }
        }
    }

    #[test]
    fn no_type_under_13px() {
        assert!(Style::ALL.iter().all(|s| s.size() >= 13));
    }

    /// The face and weight GDI actually picks for `font`, read back from a screen DC. A faked
    /// bold reads as weight 700, not 600.
    fn picked(font: &LOGFONTW) -> (String, i32) {
        unsafe {
            let hfont = CreateFontIndirectW(font);
            let dc = GetDC(None);
            let old = SelectObject(dc, hfont.into());
            let mut name = [0u16; 32];
            let n = GetTextFaceW(dc, Some(&mut name));
            let mut metrics = TEXTMETRICW::default();
            let _ = GetTextMetricsW(dc, &mut metrics);
            SelectObject(dc, old);
            ReleaseDC(None, dc);
            let _ = DeleteObject(hfont.into());
            let face = String::from_utf16_lossy(&name[..(n.max(1) - 1) as usize]);
            (face, metrics.tmWeight)
        }
    }

    fn face_name(style: Style, variable: bool) -> String {
        let face = style.face(variable);
        String::from_utf16_lossy(unsafe { face.as_wide() })
    }

    #[test]
    fn fallback_is_segoe_ui_not_a_substitute() {
        for style in Style::ALL {
            let (face, weight) = picked(&logfont(style, 96, false));
            assert_eq!(face, face_name(style, false), "{style:?}");
            assert!(face.starts_with("Segoe UI") && !face.contains("Variable"));
            assert_eq!(weight, style.weight(), "{style:?}");
        }
    }

    #[test]
    fn fonts_use_the_face_and_weight_they_ask_for() {
        let variable = has_variable();
        for style in Style::ALL {
            let (face, weight) = picked(&logfont(style, 144, variable));
            assert_eq!(face, face_name(style, variable), "{style:?}");
            assert_eq!(weight, style.weight(), "{style:?}");
        }
        let fonts = Fonts::new(144);
        assert!(Style::ALL.iter().all(|&s| !fonts.get(s).is_invalid()));
    }

    #[test]
    fn cache_rebuilds_only_on_a_new_dpi() {
        let mut cache = FontCache::new();
        let first = cache.at(96).get(Style::Body);
        assert_eq!(cache.at(96).get(Style::Body), first);
        let scaled = cache.at(144).get(Style::Body);
        assert_ne!(scaled, first);
        assert_eq!(cache.retired.as_ref().map(|f| f.dpi), Some(96));
    }

    #[test]
    fn theme_reads_again_only_for_immersive_color_set() {
        let colour = w!("ImmersiveColorSet");
        let other = w!("Policy");
        assert_eq!(
            theme_on_setting_change(LPARAM(colour.as_ptr() as isize)),
            Some(app_theme())
        );
        assert_eq!(
            theme_on_setting_change(LPARAM(other.as_ptr() as isize)),
            None
        );
        assert_eq!(theme_on_setting_change(LPARAM(0)), None);
    }
}
