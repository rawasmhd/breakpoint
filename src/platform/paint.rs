//! GDI+ painting for the popup and the form kit (DESIGN.md §7, §8, §9).
//!
//! Antialiased shapes and strokes on a GDI device context, the nudge glyphs and the close × built from
//! circles, rounded rectangles and one polygon, and the timed fill of painted buttons. Glyph
//! geometry is copied from `assets/design/*.svg` on their 20-unit grid and scaled to the size
//! asked for, so no image file ships. Colours and sizes come from the caller, which takes them
//! from the theme module.

use std::sync::Once;
use std::time::Duration;

use windows::Win32::Foundation::{COLORREF, RECT};
use windows::Win32::Graphics::Gdi::HDC;
use windows::Win32::Graphics::GdiPlus::{
    CombineModeReplace, FillModeAlternate, GdipAddPathArc, GdipAddPathLine, GdipClosePathFigure,
    GdipCreateFromHDC, GdipCreatePath, GdipCreatePen1, GdipCreateSolidFill, GdipDeleteBrush,
    GdipDeleteGraphics, GdipDeletePath, GdipDeletePen, GdipDrawEllipse, GdipDrawLine,
    GdipDrawLines, GdipDrawPath, GdipFillEllipse, GdipFillPath, GdipFillRectangle, GdipResetClip,
    GdipSetClipRect, GdipSetPenEndCap, GdipSetPenLineJoin, GdipSetPenStartCap,
    GdipSetPixelOffsetMode, GdipSetSmoothingMode, GdiplusStartup, GdiplusStartupInput, GpBrush,
    GpGraphics, GpPath, GpPen, LineCapRound, LineJoinRound, PixelOffsetModeHalf, PointF,
    SmoothingModeAntiAlias, UnitPixel,
};

use super::theme::{CLOSE_INSET, CLOSE_MARK, CLOSE_STROKE, HOVER_IN, HOVER_OUT, PRESS, Palette};

/// How often a running fade or button fill repaints.
pub const FRAME: Duration = Duration::from_millis(16);

// ---- GDI+ ----

static STARTUP: Once = Once::new();

/// Starts GDI+ for the life of the process. Safe to call more than once.
pub fn startup() {
    STARTUP.call_once(|| {
        let input = GdiplusStartupInput {
            GdiplusVersion: 1,
            ..Default::default()
        };
        let mut token = 0usize;
        // SAFETY: `input` and `token` are valid for the call. GDI+ stays up until exit, so the
        // token is never needed again.
        let _ = unsafe { GdiplusStartup(&mut token, &input, std::ptr::null_mut()) };
    });
}

/// GDI+ ARGB from a `COLORREF` (`0x00BBGGRR`), fully opaque.
pub const fn argb(colour: COLORREF) -> u32 {
    let c = colour.0;
    0xff00_0000 | (c & 0xff) << 16 | (c & 0xff00) | (c >> 16 & 0xff)
}

/// GDI+ ARGB from a `COLORREF` with `alpha` from 0 (clear) to 1 (opaque).
pub fn argb_alpha(colour: COLORREF, alpha: f32) -> u32 {
    let a = (alpha.clamp(0.0, 1.0) * 255.0).round() as u32;
    a << 24 | (argb(colour) & 0x00ff_ffff)
}

/// `a` moved toward `b` by `t` (0 to 1), channel by channel.
pub fn mix(a: COLORREF, b: COLORREF, t: f32) -> COLORREF {
    let t = t.clamp(0.0, 1.0);
    let channel = |shift: u32| {
        let (x, y) = ((a.0 >> shift & 0xff) as f32, (b.0 >> shift & 0xff) as f32);
        ((x + (y - x) * t).round() as u32) << shift
    };
    COLORREF(channel(0) | channel(8) | channel(16))
}

/// A rectangle in pixels, as floats for GDI+.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Box2 {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl From<RECT> for Box2 {
    fn from(r: RECT) -> Box2 {
        Box2 {
            x: r.left as f32,
            y: r.top as f32,
            w: (r.right - r.left) as f32,
            h: (r.bottom - r.top) as f32,
        }
    }
}

impl Box2 {
    /// This box shrunk by `by` on every side.
    pub fn inset(self, by: f32) -> Box2 {
        Box2 {
            x: self.x + by,
            y: self.y + by,
            w: (self.w - 2.0 * by).max(0.0),
            h: (self.h - 2.0 * by).max(0.0),
        }
    }
}

/// A GDI+ drawing surface on a device context, antialiased, with pixel edges on whole numbers.
pub struct Canvas {
    g: *mut GpGraphics,
}

impl Canvas {
    /// # Safety
    /// `dc` must stay valid until the canvas drops. Do not draw on `dc` with GDI meanwhile.
    pub unsafe fn new(dc: HDC) -> Option<Canvas> {
        let mut g = std::ptr::null_mut();
        // SAFETY: `dc` is valid per the contract; `g` is checked before use.
        unsafe {
            if GdipCreateFromHDC(dc, &mut g).0 != 0 || g.is_null() {
                return None;
            }
            GdipSetSmoothingMode(g, SmoothingModeAntiAlias);
            GdipSetPixelOffsetMode(g, PixelOffsetModeHalf);
        }
        Some(Canvas { g })
    }

    fn with_brush(&self, colour: COLORREF, fill: impl FnOnce(*mut GpBrush)) {
        self.with_argb_brush(argb(colour), fill);
    }

    fn with_argb_brush(&self, argb: u32, fill: impl FnOnce(*mut GpBrush)) {
        let mut brush = std::ptr::null_mut();
        // SAFETY: the brush is created, used and deleted here.
        unsafe {
            if GdipCreateSolidFill(argb, &mut brush).0 == 0 && !brush.is_null() {
                fill(brush.cast());
                GdipDeleteBrush(brush.cast());
            }
        }
    }

    fn fill_path(&self, colour: COLORREF, build: impl FnOnce(*mut GpPath)) {
        self.fill_path_argb(argb(colour), build);
    }

    fn fill_path_argb(&self, argb: u32, build: impl FnOnce(*mut GpPath)) {
        // SAFETY: `path` is live inside `with_path`; the brush is valid inside its closure.
        self.with_path(build, |path| {
            self.with_argb_brush(argb, |brush| unsafe {
                GdipFillPath(self.g, brush, path);
            })
        });
    }

    fn with_path(&self, build: impl FnOnce(*mut GpPath), then: impl FnOnce(*mut GpPath)) {
        let mut path = std::ptr::null_mut();
        // SAFETY: the path is created, used and deleted here.
        unsafe {
            if GdipCreatePath(FillModeAlternate, &mut path).0 != 0 || path.is_null() {
                return;
            }
            build(path);
            then(path);
            GdipDeletePath(path);
        }
    }

    /// A pen `width` thick with round caps and joins, for the length of `draw`.
    fn with_pen(&self, colour: COLORREF, width: f32, draw: impl FnOnce(*mut GpPen)) {
        let mut pen = std::ptr::null_mut();
        // SAFETY: the pen is created, used and deleted here.
        unsafe {
            if GdipCreatePen1(argb(colour), width, UnitPixel, &mut pen).0 != 0 || pen.is_null() {
                return;
            }
            GdipSetPenStartCap(pen, LineCapRound);
            GdipSetPenEndCap(pen, LineCapRound);
            GdipSetPenLineJoin(pen, LineJoinRound);
            draw(pen);
            GdipDeletePen(pen);
        }
    }

    pub fn fill_rect(&self, colour: COLORREF, b: Box2) {
        // SAFETY: `self.g` is live; the brush is valid inside the closure.
        self.with_brush(colour, |brush| unsafe {
            GdipFillRectangle(self.g, brush, b.x, b.y, b.w, b.h);
        });
    }

    pub fn fill_circle(&self, colour: COLORREF, cx: f32, cy: f32, r: f32) {
        // SAFETY: as in `fill_rect`.
        self.with_brush(colour, |brush| unsafe {
            GdipFillEllipse(self.g, brush, cx - r, cy - r, 2.0 * r, 2.0 * r);
        });
    }

    /// A rectangle with round corners of radius `r`, cut to half the shorter side.
    pub fn fill_round_rect(&self, colour: COLORREF, b: Box2, r: f32) {
        let r = r.min(b.w / 2.0).min(b.h / 2.0);
        if r <= 0.0 {
            return self.fill_rect(colour, b);
        }
        self.fill_path(colour, |path| round_rect_path(path, b, r));
    }

    /// As [`Canvas::fill_round_rect`], `alpha` (0 to 1) opaque over what is painted already.
    pub fn fill_round_rect_alpha(&self, colour: COLORREF, alpha: f32, b: Box2, r: f32) {
        let r = r.min(b.w / 2.0).min(b.h / 2.0).max(0.0);
        self.fill_path_argb(argb_alpha(colour, alpha), |path| {
            round_rect_path(path, b, r)
        });
    }

    /// The edge of the round rectangle `b`, `width` thick, all inside `b`, so a 1px edge on a
    /// whole-pixel box covers whole pixels. The corners follow radius `r` on the outside.
    pub fn stroke_round_rect(&self, colour: COLORREF, b: Box2, r: f32, width: f32) {
        let half = width / 2.0;
        let inner = b.inset(half);
        let r = (r - half).min(inner.w / 2.0).min(inner.h / 2.0).max(0.0);
        // SAFETY: `path` and `pen` are live inside their closures.
        self.with_path(
            |path| round_rect_path(path, inner, r),
            |path| {
                self.with_pen(colour, width, |pen| unsafe {
                    GdipDrawPath(self.g, pen, path);
                })
            },
        );
    }

    /// The edge of the circle round (`cx`, `cy`) with outer radius `r`, `width` thick, inside.
    pub fn stroke_circle(&self, colour: COLORREF, cx: f32, cy: f32, r: f32, width: f32) {
        let r = (r - width / 2.0).max(0.0);
        // SAFETY: the pen is valid inside the closure.
        self.with_pen(colour, width, |pen| unsafe {
            GdipDrawEllipse(self.g, pen, cx - r, cy - r, 2.0 * r, 2.0 * r);
        });
    }

    /// A line `width` thick with round caps.
    pub fn line(&self, colour: COLORREF, width: f32, from: (f32, f32), to: (f32, f32)) {
        // SAFETY: the pen is valid inside the closure.
        self.with_pen(colour, width, |pen| unsafe {
            GdipDrawLine(self.g, pen, from.0, from.1, to.0, to.1);
        });
    }

    /// Joined lines through `points`, `width` thick, with round caps and joins.
    pub fn polyline(&self, colour: COLORREF, width: f32, points: &[(f32, f32)]) {
        let points: Vec<PointF> = points.iter().map(|&(x, y)| PointF { X: x, Y: y }).collect();
        // SAFETY: the pen is valid inside the closure; `points` outlives the call.
        self.with_pen(colour, width, |pen| unsafe {
            GdipDrawLines(self.g, pen, points.as_ptr(), points.len() as i32);
        });
    }

    /// Runs `draw` with painting cut to `b`.
    pub fn clipped(&self, b: Box2, draw: impl FnOnce()) {
        // SAFETY: `self.g` is live; the clip is reset before return.
        unsafe { GdipSetClipRect(self.g, b.x, b.y, b.w, b.h, CombineModeReplace) };
        draw();
        // SAFETY: as above.
        unsafe { GdipResetClip(self.g) };
    }

    /// `glyph` in `colour`, scaled from its 20-unit grid to fill the square `b`.
    pub fn glyph(&self, glyph: Glyph, colour: COLORREF, b: Box2) {
        let s = b.w / GRID;
        let at = |x: f32, y: f32| (b.x + x * s, b.y + y * s);
        let rect = |r: [f32; 5]| {
            let (x, y) = at(r[0], r[1]);
            (
                Box2 {
                    x,
                    y,
                    w: r[2] * s,
                    h: r[3] * s,
                },
                r[4] * s,
            )
        };
        match glyph {
            Glyph::Stand => {
                let (cx, cy) = at(STAND_HEAD[0], STAND_HEAD[1]);
                self.fill_circle(colour, cx, cy, STAND_HEAD[2] * s);
                for r in [STAND_BODY, STAND_FLOOR] {
                    let (b, radius) = rect(r);
                    self.fill_round_rect(colour, b, radius);
                }
            }
            Glyph::Pause => {
                for r in PAUSE_BARS {
                    let (b, radius) = rect(r);
                    self.fill_round_rect(colour, b, radius);
                }
            }
            Glyph::Water => {
                let arc = drop_arc();
                let tip = at(DROP_TIP.0, DROP_TIP.1);
                let start = at(DROP_RIGHT.0, DROP_RIGHT.1);
                let (cx, cy) = at(arc.centre.0, arc.centre.1);
                let r = DROP_R * s;
                // SAFETY: `path` is the live path made by `fill_path`.
                self.fill_path(colour, |path| unsafe {
                    GdipAddPathLine(path, tip.0, tip.1, start.0, start.1);
                    GdipAddPathArc(path, cx - r, cy - r, 2.0 * r, 2.0 * r, arc.start, arc.sweep);
                    GdipClosePathFigure(path);
                });
            }
        }
    }

    /// The close ×, centred in `hit`, its strokes scaled like the rest by `scale` (dpi / 96).
    pub fn close_mark(&self, colour: COLORREF, hit: Box2, scale: f32) {
        let size = CLOSE_MARK * scale;
        let (x, y) = (hit.x + (hit.w - size) / 2.0, hit.y + (hit.h - size) / 2.0);
        let s = size / CLOSE_MARK;
        let (a, b) = (CLOSE_INSET * s, (CLOSE_MARK - CLOSE_INSET) * s);
        let width = CLOSE_STROKE * scale;
        self.line(colour, width, (x + a, y + a), (x + b, y + b));
        self.line(colour, width, (x + b, y + a), (x + a, y + b));
    }
}

impl Drop for Canvas {
    fn drop(&mut self) {
        // SAFETY: made in `new`, deleted once here.
        unsafe { GdipDeleteGraphics(self.g) };
    }
}

/// Adds the rectangle `b` with corner radius `r` (at most half a side) to `path`.
fn round_rect_path(path: *mut GpPath, b: Box2, r: f32) {
    let (right, bottom) = (b.x + b.w, b.y + b.h);
    // SAFETY: `path` is a live path, per the callers.
    unsafe {
        if r <= 0.0 {
            GdipAddPathLine(path, b.x, b.y, right, b.y);
            GdipAddPathLine(path, right, b.y, right, bottom);
            GdipAddPathLine(path, right, bottom, b.x, bottom);
        } else {
            let d = 2.0 * r;
            GdipAddPathArc(path, b.x, b.y, d, d, 180.0, 90.0);
            GdipAddPathArc(path, right - d, b.y, d, d, 270.0, 90.0);
            GdipAddPathArc(path, right - d, bottom - d, d, d, 0.0, 90.0);
            GdipAddPathArc(path, b.x, bottom - d, d, d, 90.0, 90.0);
        }
        GdipClosePathFigure(path);
    }
}

// ---- Glyphs (DESIGN.md §7), on a 20-unit grid ----

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Glyph {
    Stand,
    Pause,
    Water,
}

const GRID: f32 = 20.0;

/// stand.svg: the head `[cx, cy, r]`, then the body and the floor as `[x, y, w, h, rx]`.
const STAND_HEAD: [f32; 3] = [10.0, 4.0, 2.5];
const STAND_BODY: [f32; 5] = [8.25, 7.5, 3.5, 8.0, 1.75];
const STAND_FLOOR: [f32; 5] = [3.0, 16.75, 14.0, 2.25, 1.125];
/// pause.svg: the two bars as `[x, y, w, h, rx]`.
const PAUSE_BARS: [[f32; 5]; 2] = [
    [5.0, 3.5, 3.75, 13.0, 1.875],
    [11.25, 3.5, 3.75, 13.0, 1.875],
];
/// water.svg: `M10 1.75 L15.2 10.2 A5.9 5.9 0 1 1 4.8 10.2 Z`.
const DROP_TIP: (f32, f32) = (10.0, 1.75);
const DROP_RIGHT: (f32, f32) = (15.2, 10.2);
const DROP_LEFT: (f32, f32) = (4.8, 10.2);
const DROP_R: f32 = 5.9;

/// The drop's arc as GDI+ wants it: the circle's centre and the angles in degrees, clockwise
/// from the x axis (y points down, as in SVG).
#[derive(Debug)]
struct Arc {
    centre: (f32, f32),
    start: f32,
    sweep: f32,
}

/// The SVG arc from [`DROP_RIGHT`] to [`DROP_LEFT`], large-arc and clockwise, so round the bottom.
fn drop_arc() -> Arc {
    let half = (DROP_RIGHT.0 - DROP_LEFT.0) / 2.0;
    let drop = (DROP_R * DROP_R - half * half).sqrt();
    let centre = ((DROP_RIGHT.0 + DROP_LEFT.0) / 2.0, DROP_RIGHT.1 + drop);
    let angle = |p: (f32, f32)| (p.1 - centre.1).atan2(p.0 - centre.0).to_degrees();
    let start = angle(DROP_RIGHT);
    let end = angle(DROP_LEFT).rem_euclid(360.0);
    Arc {
        centre,
        start,
        sweep: end - start,
    }
}

// ---- Button fill (DESIGN.md §6 Buttons, §8) ----

/// How far a painted button's fill has gone: 0 at rest, 1 hovered, 2 pressed.
pub const REST: f32 = 0.0;
pub const HOVER: f32 = 1.0;
pub const PRESSED: f32 = 2.0;

/// The fill level after `dt` on the way from `level` to `target`: 150 ms from rest to hover,
/// 120 ms back, 90 ms between hover and press. At once when `animate` is false.
pub fn step_fill(level: f32, target: f32, dt: Duration, animate: bool) -> f32 {
    if !animate {
        return target;
    }
    let mut level = level;
    let mut left = dt.as_secs_f32();
    while left > 0.0 && level != target {
        let up = target > level;
        let span = if level > HOVER || (level == HOVER && up) {
            PRESS
        } else if up {
            HOVER_IN
        } else {
            HOVER_OUT
        };
        // The end of the segment `level` is in, toward `target`.
        let edge = if up {
            if level < HOVER { HOVER } else { PRESSED }
        } else if level > HOVER {
            HOVER
        } else {
            REST
        };
        let edge = if up {
            edge.min(target)
        } else {
            edge.max(target)
        };
        let need = (edge - level).abs() * span.as_secs_f32();
        if need <= left {
            level = edge;
            left -= need;
        } else {
            let rate = 1.0 / span.as_secs_f32();
            level += if up { rate * left } else { -rate * left };
            left = 0.0;
        }
    }
    level
}

/// The fill for `level`: the ground at rest, `surface-sunken` hovered, `line` pressed.
pub fn fill_colour(p: &Palette, level: f32) -> COLORREF {
    if level <= HOVER {
        mix(p.surface_raised, p.surface_sunken, level)
    } else {
        mix(p.surface_sunken, p.line, level - HOVER)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::theme::{DARK, LIGHT, rgb};

    #[test]
    fn argb_is_opaque_rgb() {
        assert_eq!(argb(rgb(0xe5484d)), 0xffe5484d);
        assert_eq!(argb_alpha(rgb(0xe5484d), 0.0), 0x00e5484d);
        assert_eq!(argb_alpha(rgb(0xe5484d), 0.5), 0x80e5484d);
    }

    #[test]
    fn mix_ends_and_middle() {
        let (a, b) = (rgb(0x000000), rgb(0xff8040));
        assert_eq!(mix(a, b, 0.0), a);
        assert_eq!(mix(a, b, 1.0), b);
        assert_eq!(mix(a, b, 0.5), rgb(0x804020));
    }

    #[test]
    fn drop_arc_runs_round_the_bottom_from_right_to_left() {
        let arc = drop_arc();
        let at = |deg: f32| {
            let r = deg.to_radians();
            (
                arc.centre.0 + DROP_R * r.cos(),
                arc.centre.1 + DROP_R * r.sin(),
            )
        };
        let close =
            |p: (f32, f32), q: (f32, f32)| (p.0 - q.0).abs() < 1e-3 && (p.1 - q.1).abs() < 1e-3;
        assert!(close(at(arc.start), DROP_RIGHT), "{arc:?}");
        assert!(close(at(arc.start + arc.sweep), DROP_LEFT), "{arc:?}");
        // Large arc, through the bottom of the circle (90°).
        assert!(arc.sweep > 180.0 && arc.start < 90.0 && arc.start + arc.sweep > 90.0);
    }

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn fill_takes_the_design_timings() {
        assert_eq!(step_fill(REST, HOVER, HOVER_IN, true), HOVER);
        assert!((step_fill(REST, HOVER, ms(75), true) - 0.5).abs() < 1e-3);
        assert_eq!(step_fill(HOVER, REST, HOVER_OUT, true), REST);
        assert!((step_fill(HOVER, REST, ms(60), true) - 0.5).abs() < 1e-3);
        assert_eq!(step_fill(HOVER, PRESSED, PRESS, true), PRESSED);
        assert_eq!(step_fill(PRESSED, HOVER, PRESS, true), HOVER);
        // Pressed to rest: 90 ms back to hover, then 120 ms to rest.
        assert_eq!(step_fill(PRESSED, REST, PRESS + HOVER_OUT, true), REST);
        assert!((step_fill(PRESSED, REST, PRESS + ms(60), true) - 0.5).abs() < 1e-3);
        // Never past the target.
        assert_eq!(step_fill(REST, HOVER, ms(1000), true), HOVER);
    }

    #[test]
    fn fill_jumps_when_animation_is_off() {
        assert_eq!(step_fill(REST, PRESSED, ms(1), false), PRESSED);
        assert_eq!(step_fill(PRESSED, REST, ms(1), false), REST);
    }

    #[test]
    fn fill_colours_are_the_button_tokens() {
        for p in [&LIGHT, &DARK] {
            assert_eq!(fill_colour(p, REST), p.surface_raised);
            assert_eq!(fill_colour(p, HOVER), p.surface_sunken);
            assert_eq!(fill_colour(p, PRESSED), p.line);
        }
    }
}
