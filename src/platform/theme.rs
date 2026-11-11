//! The design tokens as Win32 values (DESIGN.md §9).
//!
//! Every window takes its colours, sizes, fonts and timings from here. No window procedure holds
//! a colour or size literal. Sizes are at 96 DPI; [`px`] scales them for a monitor. The popup
//! follows the Windows app theme ([`app_theme`]); the dialogs stay [`Theme::Light`].

use std::time::Duration;

use windows::Win32::Foundation::{COLORREF, ERROR_SUCCESS, LPARAM};
use windows::Win32::Graphics::Gdi::{
    CLEARTYPE_QUALITY, CreateFontIndirectW, DEFAULT_CHARSET, DeleteObject, EnumFontFamiliesExW,
    GetDC, HFONT, LOGFONTW, ReleaseDC, TEXTMETRICW,
};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::Win32::UI::WindowsAndMessaging::{
    SPI_GETCLIENTAREAANIMATION, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW,
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
// `radius-md` (the popup) comes from DWM (`DWMWCP_ROUND`), and `radius-full` is half a
// shape's height, so neither needs a const here.

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

// ---- Motion (DESIGN.md §8) ----

pub const FADE_IN: Duration = Duration::from_millis(150);
pub const FADE_OUT: Duration = Duration::from_millis(100);
pub const HOVER_IN: Duration = Duration::from_millis(150);
pub const HOVER_OUT: Duration = Duration::from_millis(120);
pub const PRESS: Duration = Duration::from_millis(90);

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
