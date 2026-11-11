//! Painting a laid-out form and its menu (the controls table of issue #77, DESIGN.md §6).
//!
//! Shapes go straight onto a GDI+ [`Canvas`]; text is collected as [`Run`]s and drawn with GDI in
//! the theme fonts afterwards, as the popup does, so ClearType text sits on finished shapes. Every
//! colour comes from [`Colours`] and every size from [`super::super::theme`].

use windows::Win32::Foundation::{COLORREF, RECT, SIZE};
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateSolidBrush, DeleteDC, DeleteObject,
    FillRect, GetTextExtentPoint32W, GetTextMetricsW, HDC, SRCCOPY, SelectObject, SetBkMode,
    SetTextAlign, SetTextColor, TA_BASELINE, TA_LEFT, TEXTMETRICW, TRANSPARENT, TextOutW,
};
use windows::Win32::UI::WindowsAndMessaging::USER_DEFAULT_SCREEN_DPI as BASE_DPI;

use super::form::{Form, Laid, Look, Measure, MenuLayout, Part, rect, stepper_text};
use super::{ButtonKind, Entry, Ink, Item, Kind};
use crate::platform::paint::{Box2, Canvas, Glyph, HOVER, mix};
use crate::platform::theme::{
    BORDER_THICK, CARET, CHECK_POINTS, CHECKBOX, CHEVRON, CHEVRON_POINTS, CHEVRON_RIGHT,
    CHOICE_PAD_X, CHOICE_PAD_Y, Colours, FOCUS_RING, Fonts, HAIRLINE, KNOB, KNOB_HOT, KNOB_INSET,
    MARK_STROKE, MENU_CHECK, MENU_ITEM_PAD_X, MENU_ITEM_PAD_Y, PRESS_DROP, RADIO, RADIO_DOT,
    RADIUS_MD, RADIUS_SM, SEGMENT_PAD, SPACE_2, SPACE_3, SPACE_4, STEPPER_MARK, SWITCH_H, SWITCH_W,
    Style, px,
};

/// One line of text to draw: `top` is the top of its line box, `Style::line_height` tall.
#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    pub style: Style,
    pub colour: COLORREF,
    pub x: i32,
    pub top: i32,
    pub text: String,
}

/// Measures text with `fonts` on a device context.
pub struct GdiMeasure<'a> {
    pub dc: HDC,
    pub fonts: &'a Fonts,
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

/// Paints `w` by `h` pixels onto `dc` through a memory bitmap: the `ground` first, then the
/// shapes `paint` makes, then the text runs it returns.
///
/// # Safety
/// `dc` must be a live device context.
pub unsafe fn buffered(
    dc: HDC,
    w: i32,
    h: i32,
    ground: COLORREF,
    fonts: &Fonts,
    dpi: u32,
    paint: impl FnOnce(&Canvas, &dyn Measure, &mut Vec<Run>),
) {
    // SAFETY: every GDI object made here is freed here; `dc` is live per the contract.
    unsafe {
        let mem = CreateCompatibleDC(Some(dc));
        let bitmap = CreateCompatibleBitmap(dc, w, h);
        let old = SelectObject(mem, bitmap.into());
        paint_on(mem, w, h, ground, fonts, dpi, paint);
        let _ = BitBlt(dc, 0, 0, w, h, Some(mem), 0, 0, SRCCOPY);
        SelectObject(mem, old);
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(mem);
    }
}

/// As [`buffered`], straight onto `dc`, which should be a memory device context.
///
/// # Safety
/// `dc` must be a live device context with a bitmap at least `w` by `h`.
pub unsafe fn paint_on(
    dc: HDC,
    w: i32,
    h: i32,
    ground: COLORREF,
    fonts: &Fonts,
    dpi: u32,
    paint: impl FnOnce(&Canvas, &dyn Measure, &mut Vec<Run>),
) {
    let mut runs = Vec::new();
    // SAFETY: per the contract; the brush is freed here; the canvas drops before GDI text.
    unsafe {
        // The ground in GDI first, so text stays readable even if GDI+ is not up.
        let brush = CreateSolidBrush(ground);
        FillRect(dc, &rect(0, 0, w, h), brush);
        let _ = DeleteObject(brush.into());
        let measure = GdiMeasure { dc, fonts };
        if let Some(canvas) = Canvas::new(dc) {
            paint(&canvas, &measure, &mut runs);
        }
        draw_runs(dc, fonts, dpi, &runs);
    }
}

/// Draws `runs` with GDI in the theme fonts, each on its line box.
///
/// # Safety
/// `dc` must be a live device context.
pub unsafe fn draw_runs(dc: HDC, fonts: &Fonts, dpi: u32, runs: &[Run]) {
    // SAFETY: per the contract; fonts outlive the calls.
    unsafe {
        SetBkMode(dc, TRANSPARENT);
        SetTextAlign(dc, TA_LEFT | TA_BASELINE);
        for run in runs {
            let wide: Vec<u16> = run.text.encode_utf16().collect();
            let old = SelectObject(dc, fonts.get(run.style).into());
            let mut tm = TEXTMETRICW::default();
            let _ = GetTextMetricsW(dc, &mut tm);
            let line = px(run.style.line_height(), dpi);
            let baseline = run.top + (line - tm.tmHeight) / 2 + tm.tmAscent;
            SetTextColor(dc, run.colour);
            let _ = TextOutW(dc, run.x, baseline, &wide);
            SelectObject(dc, old);
        }
    }
}

/// What every paint call needs.
struct Pen<'a> {
    canvas: &'a Canvas,
    c: &'a Colours,
    dpi: u32,
    m: &'a dyn Measure,
    runs: &'a mut Vec<Run>,
}

impl Pen<'_> {
    /// A size token at this DPI, rounded to whole pixels.
    fn s(&self, v: i32) -> f32 {
        px(v, self.dpi) as f32
    }

    /// A stroke token at this DPI, not rounded.
    fn sf(&self, v: f32) -> f32 {
        v * self.dpi as f32 / BASE_DPI as f32
    }

    fn lh(&self, style: Style) -> i32 {
        px(style.line_height(), self.dpi)
    }

    fn text(&mut self, style: Style, colour: COLORREF, x: i32, top: i32, text: &str) {
        self.runs.push(Run {
            style,
            colour,
            x,
            top,
            text: text.to_owned(),
        });
    }

    /// `text` centred in `r`, moved down by `drop`.
    fn centred(&mut self, style: Style, colour: COLORREF, r: RECT, text: &str, drop: i32) {
        let w = self.m.width(style, text);
        let x = r.left + (r.right - r.left - w) / 2;
        let top = r.top + (r.bottom - r.top - self.lh(style)) / 2 + drop;
        self.text(style, colour, x, top, text);
    }

    /// The keyboard focus ring: `focus-ring` wide, `focus-ring` off the shape `b` of radius `r`.
    fn ring(&self, b: Box2, r: f32) {
        let w = self.s(FOCUS_RING);
        let out = w + self.s(FOCUS_RING);
        self.canvas
            .stroke_round_rect(self.c.focus, b.inset(-out), r + out, w);
    }

    /// Points from 0 to 1 on the square `b`.
    fn on_square(b: Box2, points: &[(f32, f32)]) -> Vec<(f32, f32)> {
        points
            .iter()
            .map(|&(x, y)| (b.x + x * b.w, b.y + y * b.h))
            .collect()
    }
}

/// The secondary fill for hover level `level`: raised at rest, hover at 1, press at 2.
fn fill(c: &Colours, level: f32) -> COLORREF {
    if level <= HOVER {
        mix(c.raised, c.hover, level)
    } else {
        mix(c.hover, c.press, level - HOVER)
    }
}

fn glyph_colour(c: &Colours, glyph: Glyph) -> COLORREF {
    match glyph {
        Glyph::Stand => c.stand,
        Glyph::Pause => c.pause,
        Glyph::Water => c.water,
    }
}

fn boxed(x: i32, y: i32, w: i32, h: i32) -> Box2 {
    Box2::from(rect(x, y, w, h))
}

/// Paints every item of the laid-out `form`, in list order, as `looks` says.
pub fn paint_form(
    canvas: &Canvas,
    form: &Form,
    looks: &[Look],
    c: &Colours,
    m: &dyn Measure,
    runs: &mut Vec<Run>,
) {
    let mut pen = Pen {
        canvas,
        c,
        dpi: form.dpi(),
        m,
        runs,
    };
    for ((item, laid), look) in form.items().iter().zip(form.laid()).zip(looks) {
        paint_item(&mut pen, item, laid, look);
    }
}

fn paint_item(p: &mut Pen, item: &Item, laid: &Laid, look: &Look) {
    let c = *p.c;
    let r = laid.rect;
    let b = Box2::from(r);
    let hair = p.s(HAIRLINE);
    let level = look.levels.first().copied().unwrap_or_default();
    let hover = level.min(HOVER);
    let sm = p.s(RADIUS_SM);
    let h = r.bottom - r.top;
    match &item.kind {
        Kind::Card { .. } => {
            let md = p.s(RADIUS_MD);
            p.canvas.fill_round_rect(c.raised, b, md);
            p.canvas.stroke_round_rect(c.hairline, b, md, hair);
            let side = p.s(SPACE_4);
            for &row in &laid.rows {
                let y = row as f32;
                p.canvas.fill_rect(
                    c.hairline,
                    Box2 {
                        x: b.x + side,
                        y,
                        w: b.w - 2.0 * side,
                        h: hair,
                    },
                );
            }
        }
        Kind::Band => {
            p.canvas.fill_rect(c.sunken, b);
            p.canvas.fill_rect(c.hairline, Box2 { h: hair, ..b });
        }
        Kind::Text { style, ink, .. } => {
            let colour = match ink {
                Ink::Main => c.ink,
                Ink::Muted => c.muted,
            };
            let lh = p.lh(*style);
            for (k, line) in laid.lines.iter().enumerate() {
                p.text(*style, colour, r.left, r.top + k as i32 * lh, line);
            }
        }
        Kind::Glyph(glyph) => p.canvas.glyph(*glyph, glyph_colour(&c, *glyph), b),
        Kind::Button { label, kind, .. } => {
            let label_colour = match kind {
                ButtonKind::Primary => {
                    p.canvas
                        .fill_round_rect(mix(c.action, c.action_hover, hover), b, sm);
                    c.on_action
                }
                ButtonKind::Secondary => {
                    p.canvas.fill_round_rect(fill(&c, level), b, sm);
                    p.canvas.stroke_round_rect(c.edge, b, sm, hair);
                    mix(c.ink, c.hover_ink, hover)
                }
                ButtonKind::Ghost => {
                    if level > 0.0 {
                        p.canvas.fill_round_rect_alpha(c.hover, hover, b, sm);
                    }
                    if level > HOVER {
                        p.canvas
                            .fill_round_rect_alpha(c.press, level - HOVER, b, sm);
                    }
                    mix(c.ink, c.hover_ink, hover)
                }
            };
            let drop = if level > HOVER {
                px(PRESS_DROP, p.dpi)
            } else {
                0
            };
            p.centred(Style::BodyStrong, label_colour, r, label, drop);
            if look.ring {
                p.ring(b, sm);
            }
        }
        Kind::Switch { .. } => {
            let (sw, sh) = (px(SWITCH_W, p.dpi), px(SWITCH_H, p.dpi));
            let track = boxed(r.left, r.top + (h - sh) / 2, sw, sh);
            let t = look.slide;
            let full = track.h / 2.0;
            let border = mix(mix(c.edge, c.hover_edge, hover), c.on, t);
            p.canvas
                .fill_round_rect(mix(c.raised, c.on, t), track, full);
            p.canvas.stroke_round_rect(border, track, full, hair);
            let knob = p.s(KNOB) + (p.s(KNOB_HOT) - p.s(KNOB)) * hover;
            let inset = p.s(KNOB_INSET) + p.s(KNOB) / 2.0;
            let (off, on) = (track.x + inset, track.x + track.w - inset);
            p.canvas.fill_circle(
                mix(c.edge, c.on_ink, t),
                off + (on - off) * t,
                track.y + full,
                knob / 2.0,
            );
            if look.ring {
                p.ring(track, full);
            }
        }
        Kind::Checkbox { on, label } => {
            let size = px(CHECKBOX, p.dpi);
            let square = boxed(r.left, r.top + (h - size) / 2, size, size);
            if *on {
                p.canvas.fill_round_rect(c.on, square, sm);
                let mark = Pen::on_square(square, &CHECK_POINTS);
                p.canvas.polyline(c.on_ink, p.sf(MARK_STROKE), &mark);
            } else {
                p.canvas.fill_round_rect(c.raised, square, sm);
                let edge = mix(c.edge, c.hover_edge, hover);
                p.canvas
                    .stroke_round_rect(edge, square, sm, p.sf(BORDER_THICK));
            }
            if !label.is_empty() {
                let x = r.left + size + px(SPACE_2, p.dpi);
                let top = r.top + (h - p.lh(Style::Body)) / 2;
                p.text(Style::Body, c.ink, x, top, label);
            }
            if look.ring {
                p.ring(square, sm);
            }
        }
        Kind::Day { label, on } => {
            let full = b.h / 2.0;
            let (ground, edge, ink) = if *on {
                (c.on, c.on, c.on_ink)
            } else {
                (fill(&c, level), c.edge, mix(c.ink, c.hover_ink, hover))
            };
            p.canvas.fill_round_rect(ground, b, full);
            p.canvas.stroke_round_rect(edge, b, full, hair);
            p.centred(Style::CaptionStrong, ink, r, label, 0);
            if look.ring {
                p.ring(b, full);
            }
        }
        Kind::Stepper { .. } | Kind::Time { .. } => {
            p.canvas.fill_round_rect(c.raised, b, sm);
            for (part, zone) in &laid.parts {
                if !matches!(part, Part::Minus | Part::Plus) {
                    continue;
                }
                let l = look.levels.get(part.slot()).copied().unwrap_or_default();
                let zone = Box2::from(*zone);
                if l > 0.0 {
                    p.canvas.clipped(zone, || {
                        p.canvas.fill_round_rect(fill(&c, l), b, sm);
                    });
                }
                let mark = mix(c.muted, c.hover_ink, l.min(HOVER));
                let half = p.s(STEPPER_MARK) / 2.0;
                let (cx, cy) = (zone.x + zone.w / 2.0, zone.y + zone.h / 2.0);
                let stroke = p.sf(MARK_STROKE);
                p.canvas
                    .line(mark, stroke, (cx - half, cy), (cx + half, cy));
                if *part == Part::Plus {
                    p.canvas
                        .line(mark, stroke, (cx, cy - half), (cx, cy + half));
                }
            }
            p.canvas.stroke_round_rect(c.edge, b, sm, hair);
            let (number, unit) = stepper_text(&item.kind).unwrap_or_default();
            let number = look.edit.clone().unwrap_or(number);
            let unit = if unit.is_empty() {
                String::new()
            } else {
                format!(" {unit}")
            };
            let field = laid
                .parts
                .iter()
                .find(|(part, _)| *part == Part::Field)
                .map_or(r, |&(_, f)| f);
            let nw = p.m.width(Style::BodyStrong, &number);
            let uw = p.m.width(Style::Body, &unit);
            let caret = if look.edit.is_some() {
                px(CARET, p.dpi)
            } else {
                0
            };
            let x = field.left + (field.right - field.left - nw - caret - uw) / 2;
            let top = r.top + (h - p.lh(Style::Body)) / 2;
            p.text(Style::BodyStrong, c.ink, x, top, &number);
            if look.edit.is_some() {
                let tall = px(Style::Body.size(), p.dpi);
                let y = top + (p.lh(Style::Body) - tall) / 2;
                p.canvas.fill_rect(c.ink, boxed(x + nw, y, caret, tall));
            }
            if !unit.is_empty() {
                p.text(Style::Body, c.ink, x + nw + caret, top, &unit);
            }
            if look.ring {
                p.ring(b, sm);
            }
        }
        Kind::Segmented { options, selected } => {
            let track = sm + p.s(SEGMENT_PAD);
            p.canvas.fill_round_rect(c.sunken, b, track);
            for (part, seg) in &laid.parts {
                let Part::Segment(k) = *part else { continue };
                let label = options.get(k).map_or("", String::as_str);
                let ink = if k == *selected {
                    let sb = Box2::from(*seg);
                    p.canvas.fill_round_rect(c.chosen, sb, sm);
                    p.canvas.stroke_round_rect(c.chosen_edge, sb, sm, hair);
                    c.on_chosen
                } else {
                    let l = look.levels.get(k).copied().unwrap_or_default();
                    mix(c.muted, c.ink, l.min(HOVER))
                };
                p.centred(Style::CaptionStrong, ink, *seg, label, 0);
            }
            if look.ring {
                p.ring(b, track);
            }
        }
        Kind::DropDown {
            entries,
            current,
            other,
        } => {
            p.canvas.fill_round_rect(fill(&c, hover), b, sm);
            if look.open {
                p.canvas.stroke_round_rect(c.ink, b, sm, hair);
                p.canvas
                    .stroke_round_rect(c.ink, b.inset(-hair), sm + hair, hair);
            } else {
                p.canvas.stroke_round_rect(c.edge, b, sm, hair);
            }
            let shown = current
                .and_then(|i| entries.get(i))
                .map_or(other.as_str(), |e| e.name.as_str());
            let top = r.top + (h - p.lh(Style::Body)) / 2;
            let ink = mix(c.ink, c.hover_ink, hover);
            p.text(Style::Body, ink, r.left + px(SPACE_3, p.dpi), top, shown);
            let size = px(CHEVRON, p.dpi);
            let chevron = boxed(
                r.right - px(CHEVRON_RIGHT, p.dpi) - size,
                r.top + (h - size) / 2,
                size,
                size,
            );
            let mark = Pen::on_square(chevron, &CHEVRON_POINTS);
            let chevron_ink = mix(c.muted, c.hover_ink, hover);
            p.canvas.polyline(chevron_ink, p.sf(MARK_STROKE), &mark);
            if look.ring {
                p.ring(b, sm);
            }
        }
        Kind::Choice {
            title,
            note,
            selected,
            aside,
            ..
        } => {
            let md = p.s(RADIUS_MD);
            p.canvas.fill_round_rect(c.raised, b, md);
            if *selected {
                p.canvas.stroke_round_rect(c.on, b, md, hair);
                p.canvas
                    .stroke_round_rect(c.on, b.inset(-hair), md + hair, hair);
            } else {
                p.canvas.stroke_round_rect(c.hairline, b, md, hair);
            }
            let (pad_x, pad_y) = (px(CHOICE_PAD_X, p.dpi), px(CHOICE_PAD_Y, p.dpi));
            let radio = px(RADIO, p.dpi);
            let top = r.top + pad_y;
            let title_lh = p.lh(Style::BodyStrong);
            let (cx, cy) = (
                (r.left + pad_x) as f32 + radio as f32 / 2.0,
                (top + (title_lh - radio) / 2) as f32 + radio as f32 / 2.0,
            );
            let edge = if *selected {
                c.on
            } else {
                mix(c.edge, c.hover_edge, hover)
            };
            p.canvas
                .stroke_circle(edge, cx, cy, radio as f32 / 2.0, p.sf(BORDER_THICK));
            if *selected {
                p.canvas.fill_circle(c.on, cx, cy, p.s(RADIO_DOT) / 2.0);
            }
            let x = r.left + pad_x + radio + px(SPACE_3, p.dpi);
            p.text(Style::BodyStrong, c.ink, x, top, title);
            let caption_lh = p.lh(Style::Caption);
            let notes = if laid.lines.is_empty() {
                std::slice::from_ref(note)
            } else {
                laid.lines.as_slice()
            };
            for (k, line) in (0..).zip(notes) {
                p.text(
                    Style::Caption,
                    c.muted,
                    x,
                    top + title_lh + k * caption_lh,
                    line,
                );
            }
            if let Some(aside) = aside {
                let (aw, ah) = (px(aside.w, p.dpi), px(aside.h, p.dpi));
                let at = boxed(r.right - pad_x - aw, r.top + (h - ah) / 2, aw, ah);
                (aside.paint)(p.canvas, at, &c, p.dpi);
            }
            if look.ring {
                p.ring(b, md);
            }
        }
    }
}

/// Paints the open menu on its own window's client area: `surface-raised`, the outline when
/// DWM draws no border (`outline`), the highlighted row, the check at the current entry, and
/// each entry's name and note.
#[allow(clippy::too_many_arguments)]
pub fn paint_menu(
    canvas: &Canvas,
    layout: &MenuLayout,
    entries: &[Entry],
    current: Option<usize>,
    hot: Option<usize>,
    outline: bool,
    c: &Colours,
    dpi: u32,
    runs: &mut Vec<Run>,
) {
    let s = |v: i32| px(v, dpi);
    let all = boxed(0, 0, layout.w, layout.h);
    canvas.fill_rect(c.raised, all);
    if outline {
        canvas.stroke_round_rect(c.hairline, all, 0.0, s(HAIRLINE) as f32);
    }
    let name_lh = s(Style::BodyStrong.line_height());
    let check = s(MENU_CHECK);
    for (k, (row, entry)) in layout.rows.iter().zip(entries).enumerate() {
        let (ink, muted) = if hot == Some(k) {
            canvas.fill_round_rect(c.hot_row, Box2::from(*row), s(RADIUS_SM) as f32);
            (c.on_hot_row, c.on_hot_row)
        } else {
            (c.ink, c.muted)
        };
        let top = row.top + s(MENU_ITEM_PAD_Y);
        let left = row.left + s(MENU_ITEM_PAD_X);
        if current == Some(k) {
            let square = boxed(left, top + (name_lh - check) / 2, check, check);
            let stroke = MARK_STROKE * dpi as f32 / BASE_DPI as f32;
            canvas.polyline(ink, stroke, &Pen::on_square(square, &CHECK_POINTS));
        }
        let x = left + check + s(SPACE_2);
        runs.push(Run {
            style: Style::BodyStrong,
            colour: ink,
            x,
            top,
            text: entry.name.clone(),
        });
        if let Some(note) = &entry.note {
            runs.push(Run {
                style: Style::Caption,
                colour: muted,
                x,
                top: top + name_lh,
                text: note.clone(),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::theme::{DARK, LIGHT, Theme};

    #[test]
    fn secondary_fill_is_the_button_tokens() {
        for theme in [Theme::Light, Theme::Dark] {
            let c = Colours::of(theme);
            let p = theme.palette();
            assert_eq!(fill(&c, 0.0), p.surface_raised);
            assert_eq!(fill(&c, 1.0), p.surface_sunken);
            assert_eq!(fill(&c, 2.0), p.line);
        }
        assert_eq!(Colours::of(Theme::Light).on_ink, LIGHT.surface);
        assert_eq!(Colours::of(Theme::Dark).on, DARK.ink);
    }

    #[test]
    fn marks_land_on_their_square() {
        let b = boxed(10, 20, 100, 100);
        let points = Pen::on_square(b, &[(0.0, 0.0), (1.0, 0.5)]);
        assert_eq!(points, [(10.0, 20.0), (110.0, 70.0)]);
    }
}
