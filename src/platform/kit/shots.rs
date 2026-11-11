//! For a look by eye: `BP_SHOTS=<folder> cargo test kit::shots -- --ignored` writes a demo form
//! with every control in every state to BMPs there: light, dark and high contrast (the High
//! Contrast Black colours), at 100, 125, 150 and 200 %. Then it opens a live kit window in the
//! current theme at the monitor's scale and captures it with its frame, and its open menu.
//!
//! The demo's places are test data at 96 DPI; the controls take every size from the theme.

use std::path::Path;
use std::rc::Rc;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPFILEHEADER, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CreateCompatibleBitmap,
    CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetDIBits, HBITMAP, HDC,
    ReleaseDC, SRCCOPY, SelectObject,
};
use windows::Win32::Storage::Xps::{PRINT_WINDOW_FLAGS, PrintWindow};
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetThreadDpiAwarenessContext,
};
use windows::Win32::UI::Input::KeyboardAndMouse::VK_SPACE;
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetWindowRect, MSG, PM_REMOVE, PW_RENDERFULLCONTENT, PeekMessageW,
    SendMessageW, TranslateMessage, WM_KEYDOWN,
};

use super::draw::{GdiMeasure, paint_form, paint_menu, paint_on};
use super::form::{Form, Look, Part, menu_layout};
use super::window::{Spec, open};
use super::{Aside, ButtonKind, Entry, Ink, Item, Kind, Place};
use crate::platform::paint::{self, Box2, Glyph, HOVER, PRESSED};
use crate::platform::theme::{
    self, Colours, Fonts, MENU_GAP, SPACE_4, SPACE_6, Style, Theme, high_contrast_black, px,
};

/// How a demo item is shown.
#[derive(Clone, Copy, Debug)]
enum Shot {
    Rest,
    Hover(Part),
    Press(Part),
    Ring,
    Open,
    Edit(&'static str),
}

const LABEL_W: i32 = 116;
const COL: i32 = 128;
const ROW: i32 = 48;
const WIDE_ROW: i32 = 82;
const HEADS: [&str; 6] = [
    "Rest",
    "Hover",
    "Press",
    "On, open or typing",
    "On, hover",
    "Focus",
];
const WIDTH: i32 = SPACE_6 + LABEL_W + 6 * COL + SPACE_6;
const DROP_ID: u16 = 900;

fn col(k: i32) -> i32 {
    SPACE_6 + LABEL_W + k * COL
}

/// The wizard's timeline, roughly, to show the paint hook: a baseline, `stand` ticks every 30
/// minutes and `pause` ticks every 60, over a nine-hour day.
fn timeline() -> Aside {
    Aside {
        w: 150,
        h: 24,
        paint: Rc::new(|canvas, b: Box2, c: &Colours, dpi| {
            let s = |v: f32| v * dpi as f32 / 96.0;
            let mid = b.y + b.h / 2.0;
            canvas.fill_rect(
                c.hairline,
                Box2 {
                    y: mid - s(1.0),
                    h: s(2.0),
                    ..b
                },
            );
            for minute in (30..540).step_by(30) {
                let x = b.x + b.w * minute as f32 / 540.0 - s(1.5);
                let (colour, tall) = if minute % 60 == 0 {
                    (c.pause, s(20.0))
                } else {
                    (c.stand, s(12.0))
                };
                let tick = Box2 {
                    x,
                    y: mid - tall / 2.0,
                    w: s(3.0),
                    h: tall,
                };
                canvas.fill_round_rect(colour, tick, s(1.5));
            }
        }),
    }
}

fn entries() -> Vec<Entry> {
    vec![
        Entry::new("Gentle", Some("Stand every 45 min, pause every 90")),
        Entry::new("Balanced", Some("Stand every 30 min, pause every 60")),
        Entry::new("Strict", Some("Stand every 20 min, pause every 40")),
    ]
}

/// The demo form, with how each item is shown, and its client height at 96 DPI.
fn demo() -> (Vec<(Item, Shot)>, i32) {
    let mut out: Vec<(Item, Shot)> = Vec::new();
    let mut id = 0u16;
    let mut add = |out: &mut Vec<(Item, Shot)>, at: Place, kind: Kind, shot: Shot| {
        id += 1;
        out.push((Item::new(id, at, kind), shot));
    };
    let text = |t: &str, style, ink| Kind::text(t, style, ink);
    let mut y = SPACE_6;
    for (k, head) in HEADS.iter().enumerate() {
        add(
            &mut out,
            Place::at(col(k as i32), y),
            text(head, Style::Caption, Ink::Muted),
            Shot::Rest,
        );
    }
    y += Style::Caption.line_height() + SPACE_4;

    // One row per control: the label, then the control in each column's state.
    let mut row = |out: &mut Vec<(Item, Shot)>,
                   y: &mut i32,
                   label: &str,
                   h: i32,
                   cells: &[(i32, Kind, Shot)]| {
        let top = *y + (ROW - Style::Body.line_height()) / 2;
        add(
            out,
            Place::at(SPACE_6, top),
            text(label, Style::Body, Ink::Main),
            Shot::Rest,
        );
        for (k, kind, shot) in cells {
            add(
                out,
                Place::at(col(*k), *y + (ROW - h) / 2),
                kind.clone(),
                *shot,
            );
        }
        *y += ROW;
    };
    let states = |off: Kind, on: Kind| {
        vec![
            (0, off.clone(), Shot::Rest),
            (1, off.clone(), Shot::Hover(Part::Body)),
            (2, off.clone(), Shot::Press(Part::Body)),
            (3, on.clone(), Shot::Rest),
            (4, on, Shot::Hover(Part::Body)),
            (5, off, Shot::Ring),
        ]
    };
    let button = |label: &str, kind| {
        let b = Kind::button(label, kind);
        vec![
            (0, b.clone(), Shot::Rest),
            (1, b.clone(), Shot::Hover(Part::Body)),
            (2, b.clone(), Shot::Press(Part::Body)),
            (5, b, Shot::Ring),
        ]
    };
    row(
        &mut out,
        &mut y,
        "Primary",
        32,
        &button("Save", ButtonKind::Primary),
    );
    row(
        &mut out,
        &mut y,
        "Secondary",
        32,
        &button("Cancel", ButtonKind::Secondary),
    );
    row(
        &mut out,
        &mut y,
        "Ghost",
        32,
        &button("Skip", ButtonKind::Ghost),
    );
    row(
        &mut out,
        &mut y,
        "Switch",
        22,
        &states(Kind::Switch { on: false }, Kind::Switch { on: true }),
    );
    let check = |on| Kind::Checkbox {
        on,
        label: "Start".into(),
    };
    row(
        &mut out,
        &mut y,
        "Checkbox",
        20,
        &states(check(false), check(true)),
    );
    let day = |on| Kind::Day {
        label: "Mon".into(),
        on,
    };
    row(
        &mut out,
        &mut y,
        "Day pill",
        32,
        &states(day(false), day(true)),
    );
    let every = Kind::Stepper {
        value: 30,
        min: 10,
        max: 240,
        step: 1,
        unit: "min".into(),
    };
    row(
        &mut out,
        &mut y,
        "Stepper",
        32,
        &[
            (0, every.clone(), Shot::Rest),
            (1, every.clone(), Shot::Hover(Part::Minus)),
            (2, every.clone(), Shot::Press(Part::Plus)),
            (3, every.clone(), Shot::Edit("45")),
            (5, every, Shot::Ring),
        ],
    );
    let time = Kind::Time {
        minutes: 8 * 60,
        step: 15,
    };
    row(
        &mut out,
        &mut y,
        "Time stepper",
        32,
        &[
            (0, time.clone(), Shot::Rest),
            (1, time.clone(), Shot::Hover(Part::Plus)),
            (2, time.clone(), Shot::Press(Part::Minus)),
            (3, time.clone(), Shot::Edit("8:3")),
            (5, time, Shot::Ring),
        ],
    );
    let seg = |selected| Kind::Segmented {
        options: vec!["Soft".into(), "Firm".into()],
        selected,
    };
    row(
        &mut out,
        &mut y,
        "Segmented",
        32,
        &[
            (0, seg(0), Shot::Rest),
            (1, seg(0), Shot::Hover(Part::Segment(1))),
            (2, seg(0), Shot::Press(Part::Segment(1))),
            (3, seg(1), Shot::Rest),
            (4, seg(1), Shot::Hover(Part::Segment(0))),
            (5, seg(0), Shot::Ring),
        ],
    );
    let drop = |current| Kind::DropDown {
        entries: entries(),
        current,
        other: "Custom".into(),
    };
    let dy = y + (ROW - 32) / 2;
    add(
        &mut out,
        Place::at(SPACE_6, y + (ROW - 20) / 2),
        text("Drop-down", Style::Body, Ink::Main),
        Shot::Rest,
    );
    add(
        &mut out,
        Place::new(col(0), dy, COL - 8, 0),
        drop(Some(1)),
        Shot::Rest,
    );
    add(
        &mut out,
        Place::new(col(1), dy, COL - 8, 0),
        drop(None),
        Shot::Hover(Part::Body),
    );
    let open_at = out.len();
    add(
        &mut out,
        Place::new(col(3), dy, COL - 8, 0),
        drop(Some(1)),
        Shot::Open,
    );
    add(
        &mut out,
        Place::new(col(5), dy, COL - 8, 0),
        drop(Some(1)),
        Shot::Ring,
    );
    out[open_at].0.id = DROP_ID;
    // Room for the open menu under the drop-down.
    y += ROW + 3 * 54 + 2 * 4;

    let choice = |group, title: &str, selected| Kind::Choice {
        group,
        title: title.into(),
        note: "Stand 30 min, pause 60".into(),
        selected,
        aside: Some(timeline()),
    };
    let wide = 3 * COL - 8;
    let cy = |y: i32| y + (WIDE_ROW - 66) / 2;
    add(
        &mut out,
        Place::at(SPACE_6, y + 8),
        text("Selectable card", Style::Body, Ink::Main),
        Shot::Rest,
    );
    add(
        &mut out,
        Place::new(col(0), cy(y), wide, 0),
        choice(1, "Gentle", false),
        Shot::Rest,
    );
    add(
        &mut out,
        Place::new(col(3), cy(y), wide, 0),
        choice(2, "Gentle", false),
        Shot::Hover(Part::Body),
    );
    y += WIDE_ROW;
    add(
        &mut out,
        Place::new(col(0), cy(y), wide, 0),
        choice(3, "Balanced", true),
        Shot::Rest,
    );
    add(
        &mut out,
        Place::new(col(3), cy(y), wide, 0),
        choice(4, "Balanced", true),
        Shot::Ring,
    );
    y += WIDE_ROW + SPACE_4;

    // A card of text styles and glyphs, with hairlines between its rows.
    let card_w = WIDTH - 2 * SPACE_6;
    let rows = [56, 44, 44];
    add(
        &mut out,
        Place::new(SPACE_6, y, card_w, rows.iter().sum()),
        Kind::Card {
            rows: vec![rows[0], rows[0] + rows[1]],
        },
        Shot::Rest,
    );
    let inner = SPACE_6 + SPACE_4;
    add(
        &mut out,
        Place::at(inner, y + 10),
        text("Pick a rhythm", Style::TitleLg, Ink::Main),
        Shot::Rest,
    );
    add(
        &mut out,
        Place::at(inner + 260, y + 14),
        text("Stand up and move", Style::Title, Ink::Main),
        Shot::Rest,
    );
    let mut x = inner;
    let line = y + rows[0] + 12;
    for (t, style) in [
        ("Body", Style::Body),
        ("Body strong", Style::BodyStrong),
        ("Caption", Style::Caption),
        ("Caption strong 1:59", Style::CaptionStrong),
    ] {
        add(
            &mut out,
            Place::at(x, line),
            text(t, style, Ink::Main),
            Shot::Rest,
        );
        x += 150;
    }
    add(
        &mut out,
        Place::at(x, line),
        text("Muted note", Style::Caption, Ink::Muted),
        Shot::Rest,
    );
    let line = y + rows[0] + rows[1] + 12;
    for (k, glyph) in [Glyph::Stand, Glyph::Water, Glyph::Pause]
        .into_iter()
        .enumerate()
    {
        add(
            &mut out,
            Place::at(inner + k as i32 * 32, line),
            Kind::Glyph(glyph),
            Shot::Rest,
        );
    }
    add(
        &mut out,
        // One line reserved: the second line grows the card and moves the footer down.
        Place::new(inner + 110, line, 560, Style::Caption.line_height()),
        text(
            "Always gentle; rides along with the next nudge. A long note wraps at spaces to the width its place gives it.",
            Style::Caption,
            Ink::Muted,
        ),
        Shot::Rest,
    );
    y += rows.iter().sum::<i32>() + SPACE_6;

    // The footer band with the dialog buttons.
    let band = 32 + 2 * SPACE_4;
    add(
        &mut out,
        Place::new(0, y, WIDTH, band),
        Kind::Band,
        Shot::Rest,
    );
    let mut save = Kind::button("Save", ButtonKind::Primary);
    if let Kind::Button { default, .. } = &mut save {
        *default = true;
    }
    let mut cancel = Kind::button("Cancel", ButtonKind::Secondary);
    if let Kind::Button { cancel: c, .. } = &mut cancel {
        *c = true;
    }
    add(
        &mut out,
        Place::new(WIDTH - SPACE_6 - 80 - 8 - 80, y + SPACE_4, 80, 0),
        save,
        Shot::Rest,
    );
    add(
        &mut out,
        Place::new(WIDTH - SPACE_6 - 80, y + SPACE_4, 80, 0),
        cancel,
        Shot::Rest,
    );
    (out, y + band)
}

fn apply(look: &mut Look, shot: Shot) {
    let mut set = |part: Part, level: f32| {
        if let Some(l) = look.levels.get_mut(part.slot()) {
            *l = level;
        }
    };
    match shot {
        Shot::Rest => {}
        Shot::Hover(part) => set(part, HOVER),
        Shot::Press(part) => set(part, PRESSED),
        Shot::Ring => look.ring = true,
        Shot::Open => look.open = true,
        Shot::Edit(text) => look.edit = Some(text.to_owned()),
    }
}

/// Saves the bitmap selected into `dc` (`w` by `h`) as a 32-bit BMP.
unsafe fn save_bmp(dc: HDC, bitmap: HBITMAP, w: i32, h: i32, path: &Path) {
    let mut info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut pixels = vec![0u8; (w * h * 4) as usize];
    unsafe {
        GetDIBits(
            dc,
            bitmap,
            0,
            h as u32,
            Some(pixels.as_mut_ptr() as _),
            &mut info,
            DIB_RGB_COLORS,
        );
    }
    let offset = (size_of::<BITMAPFILEHEADER>() + size_of::<BITMAPINFOHEADER>()) as u32;
    let mut file = Vec::new();
    file.extend_from_slice(b"BM");
    file.extend_from_slice(&(offset + pixels.len() as u32).to_le_bytes());
    file.extend_from_slice(&0u32.to_le_bytes());
    file.extend_from_slice(&offset.to_le_bytes());
    let header = unsafe {
        std::slice::from_raw_parts(
            &info.bmiHeader as *const BITMAPINFOHEADER as *const u8,
            size_of::<BITMAPINFOHEADER>(),
        )
    };
    file.extend_from_slice(header);
    file.extend_from_slice(&pixels);
    std::fs::write(path, file).unwrap();
}

/// A memory bitmap `w` by `h` to draw on, saved to `path` when `draw` returns.
fn render(w: i32, h: i32, path: &Path, draw: impl FnOnce(HDC)) {
    unsafe {
        let screen = GetDC(None);
        let dc = CreateCompatibleDC(Some(screen));
        let bitmap = CreateCompatibleBitmap(screen, w, h);
        let old = SelectObject(dc, bitmap.into());
        draw(dc);
        SelectObject(dc, old);
        save_bmp(dc, bitmap, w, h, path);
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(dc);
        ReleaseDC(None, screen);
    }
}

fn pump(for_ms: u64) {
    let end = Instant::now() + Duration::from_millis(for_ms);
    let mut msg = MSG::default();
    while Instant::now() < end {
        unsafe {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Captures `hwnd` with its frame, as DWM shows it.
fn capture(hwnd: HWND, path: &Path) {
    let mut r = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut r).unwrap() };
    let (w, h) = (r.right - r.left, r.bottom - r.top);
    render(w, h, path, |dc| unsafe {
        let _ = PrintWindow(hwnd, dc, PRINT_WINDOW_FLAGS(PW_RENDERFULLCONTENT));
    });
}

#[test]
#[ignore = "writes BMPs and shows a window"]
fn shots() {
    let Ok(folder) = std::env::var("BP_SHOTS") else {
        eprintln!("set BP_SHOTS to a folder for the shots");
        return;
    };
    let folder = std::path::PathBuf::from(folder);
    unsafe {
        SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    paint::startup();
    let (demo, height) = demo();
    let schemes = [
        ("light", Colours::of(Theme::Light)),
        ("dark", Colours::of(Theme::Dark)),
        ("contrast", Colours::high_contrast(high_contrast_black)),
    ];
    for (name, c) in schemes {
        for dpi in [96, 120, 144, 192] {
            let fonts = Fonts::new(dpi);
            let mut form = Form::new(demo.iter().map(|(item, _)| item.clone()).collect());
            unsafe {
                let screen = GetDC(None);
                form.lay_out(
                    dpi,
                    &GdiMeasure {
                        dc: screen,
                        fonts: &fonts,
                    },
                );
                ReleaseDC(None, screen);
            }
            let (w, h) = (px(WIDTH, dpi), form.height(height));
            let path = folder.join(format!("kit-{name}-{dpi}.bmp"));
            render(w, h, &path, |dc| unsafe {
                let mut looks = form.looks();
                for (look, (_, shot)) in looks.iter_mut().zip(&demo) {
                    apply(look, *shot);
                }
                paint_on(dc, w, h, c.ground, &fonts, dpi, |canvas, m, runs| {
                    paint_form(canvas, &form, &looks, &c, m, runs);
                });
                // The open drop-down's menu, as its own window would show it, under the field.
                let i = form.index(DROP_ID).unwrap();
                let field = form.laid()[i].rect;
                let entries = entries();
                let layout = menu_layout(
                    &entries,
                    field.right - field.left,
                    dpi,
                    &GdiMeasure { dc, fonts: &fonts },
                );
                let menu_dc = CreateCompatibleDC(Some(dc));
                let menu_bitmap = CreateCompatibleBitmap(dc, layout.w, layout.h);
                let old = SelectObject(menu_dc, menu_bitmap.into());
                paint_on(
                    menu_dc,
                    layout.w,
                    layout.h,
                    c.raised,
                    &fonts,
                    dpi,
                    |canvas, _, runs| {
                        paint_menu(
                            canvas,
                            &layout,
                            &entries,
                            Some(1),
                            Some(0),
                            true,
                            &c,
                            dpi,
                            runs,
                        );
                    },
                );
                let top = field.bottom + px(MENU_GAP, dpi);
                let _ = BitBlt(
                    dc,
                    field.left,
                    top,
                    layout.w,
                    layout.h,
                    Some(menu_dc),
                    0,
                    0,
                    SRCCOPY,
                );
                SelectObject(menu_dc, old);
                let _ = DeleteObject(menu_bitmap.into());
                let _ = DeleteDC(menu_dc);
            });
        }
    }

    // A live window in the current theme, at the monitor's scale, with its frame.
    let items = demo.into_iter().map(|(item, _)| item).collect();
    let spec = Spec {
        caption: "breakpoint kit".into(),
        width: WIDTH,
        height,
        items,
        at: Some(POINT { x: 40, y: 40 }),
        owner: None,
    };
    let handle = open(spec, |_, _| {}).unwrap();
    handle.show();
    pump(400);
    capture(handle.hwnd(), &folder.join("kit-live.bmp"));
    handle.focus(DROP_ID);
    unsafe {
        SendMessageW(
            handle.hwnd(),
            WM_KEYDOWN,
            Some(WPARAM(VK_SPACE.0.into())),
            Some(LPARAM(0)),
        );
    }
    pump(400);
    capture(handle.hwnd(), &folder.join("kit-live-open.bmp"));
    let menu = super::window::menu_of(handle).expect("the menu window is open");
    capture(menu, &folder.join("kit-live-menu.bmp"));
    eprintln!(
        "live: theme {:?}, high contrast {}",
        theme::app_theme(),
        theme::high_contrast()
    );
    handle.close();
}
