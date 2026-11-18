//! The flyout's content for every SPEC §7 state, its layout at every scale, its placement for
//! every taskbar edge, its screen-reader tree, and the live window: a borderless popup that
//! closes on Esc, on losing activation and after an action. No test assumes a screen size.
//! `BP_SHOTS=<folder> cargo test flyout::tests::shots -- --ignored` writes BMPs for a look.

use std::path::Path;

use breakpoint::core::config::Config;
use breakpoint::core::quiet::{Switch, WorkWeek, switch_item};
use breakpoint::core::time::LocalTime;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::Graphics::Gdi::{GetDC, ReleaseDC};
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForWindow, SetThreadDpiAwarenessContext,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{VK_ESCAPE, VK_RETURN, VK_SPACE, VK_TAB};
use windows::Win32::UI::WindowsAndMessaging::{
    GWL_EXSTYLE, GWL_STYLE, GetClientRect, GetWindowLongW, SendMessageW, WA_INACTIVE, WM_ACTIVATE,
    WM_KEYDOWN, WS_CAPTION, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

use super::*;
use crate::platform::kit::draw::{GdiMeasure, paint_form, paint_on};
use crate::platform::kit::form::{Form, Measure};
use crate::platform::kit::shots::{pump, render};
use crate::platform::kit::uia::Tree;
use crate::platform::theme::{Colours, Fonts, Theme, high_contrast_black};

const SCALES: [u32; 3] = [96, 144, 192];

/// 2026-11-01 is a Sunday, so 2026-11-03 is a Tuesday and 2026-11-05 a Thursday.
fn at(s: &str) -> LocalTime {
    LocalTime::parse_rfc3339(&format!("{s}+03:00")).unwrap()
}

fn week() -> WorkWeek {
    WorkWeek::from_config(&Config::default())
}

fn prompt(nudge: Nudge, water: bool, snooze: bool) -> Prompt {
    Prompt {
        nudge,
        title: nudge.title(),
        line: String::new(),
        length: Duration::from_secs(300),
        snooze,
        water,
    }
}

/// The flyout at `now` with the Off or Overtime `switch` in force, nothing pending.
fn state(now: &str, switch: Option<Switch>) -> Content {
    content(
        &switch_item(&week(), switch, at(now)),
        None,
        Duration::ZERO,
        (7, 2),
    )
}

// ---- Content (SPEC §7) ----

#[test]
fn every_section_7_state_shows_its_status_switch_and_note() {
    let off = |s: &str| Some(Switch::Off { until: at(s) });
    let overtime = |s: &str| Some(Switch::Overtime { until: at(s) });
    let rows = [
        // In hours, nudging.
        (
            state("2026-11-03T10:00:00", None),
            (true, "On until 17:00", "Turn off until tomorrow 08:00."),
        ),
        // In hours, Off.
        (
            state("2026-11-03T10:00:00", off("2026-11-04T08:00:00")),
            (false, "Off until tomorrow 08:00", "Turn back on now."),
        ),
        // Outside hours, quiet.
        (
            state("2026-11-05T18:00:00", None),
            (
                false,
                "Quiet until Sunday 08:00",
                "Outside work hours. Turn on until midnight.",
            ),
        ),
        // Outside hours, Off still in force.
        (
            state("2026-11-05T18:00:00", off("2026-11-08T08:00:00")),
            (
                false,
                "Off until Sunday 08:00",
                "Outside work hours. Turn on until midnight.",
            ),
        ),
        // Overtime, outside hours.
        (
            state("2026-11-05T18:00:00", overtime("2026-11-06T00:00:00")),
            (
                true,
                "On until midnight",
                "Overtime. Turn off to be quiet again.",
            ),
        ),
        // Overtime, in hours.
        (
            state("2026-11-03T09:00:00", overtime("2026-11-04T00:00:00")),
            (
                true,
                "On until midnight",
                "Overtime. Turn off until tomorrow 08:00.",
            ),
        ),
    ];
    for (c, (on, status, note)) in rows {
        assert_eq!(
            (c.on, c.status.as_str(), c.note.as_str()),
            (on, status, note)
        );
        assert_eq!((c.taken, c.skipped), (7, 2));
        assert_eq!(c.pending, None);
        // The switch and the dot follow the state.
        let items = items(&c);
        let switch = items.iter().find(|i| i.id == SWITCH).unwrap();
        assert_eq!(switch.kind, Kind::Switch { on });
        assert!(items.iter().any(|i| i.kind == Kind::Dot { on }));
    }
}

#[test]
fn a_pending_occurrence_brings_its_card() {
    let item = switch_item(&week(), None, at("2026-11-03T10:00:00"));
    let left = Duration::from_secs(119);
    let c = content(&item, Some(&prompt(Nudge::Pause, true, true)), left, (0, 0));
    assert_eq!(
        c.pending,
        Some(Pending {
            nudge: Nudge::Pause,
            left,
            water: true,
            snooze: true
        })
    );
    let items = items(&c);
    let texts: Vec<&str> = items
        .iter()
        .filter_map(|i| match &i.kind {
            Kind::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert!(texts.contains(&"Pause pending"));
    assert!(texts.contains(&WATER_LINE), "the riding Water line");
    assert!(items.iter().any(|i| i.kind == Kind::Glyph(Glyph::Pause)));
    assert!(items.iter().any(|i| i.kind == Kind::Glyph(Glyph::Water)));
    let countdown = items.iter().find(|i| i.id == COUNTDOWN).unwrap();
    assert_eq!(
        countdown.kind,
        Kind::Spans {
            spans: vec![
                Span::new("Away ", Style::Caption, Ink::Muted),
                Span::new("1:59", Style::CaptionStrong, Ink::Main),
                Span::new(" more and it counts", Style::Caption, Ink::Muted),
            ],
            right: false
        }
    );
    // Tab order, top to bottom: the switch, Snooze, Skip, Settings…, Quit.
    let form = Form::new(items.clone());
    let stops: Vec<Id> = form.tab_stops().into_iter().map(|i| items[i].id).collect();
    assert_eq!(stops, [SWITCH, SNOOZE, SKIP, SETTINGS, QUIT]);
}

#[test]
fn snooze_goes_once_used_and_water_only_when_it_rode() {
    let item = switch_item(&week(), None, at("2026-11-03T10:00:00"));
    let c = content(
        &item,
        Some(&prompt(Nudge::Stand, false, false)),
        Duration::from_secs(60),
        (1, 0),
    );
    let items = items(&c);
    assert!(items.iter().all(|i| i.id != SNOOZE));
    assert!(items.iter().any(|i| i.id == SKIP));
    assert!(items.iter().all(|i| i.kind != Kind::Glyph(Glyph::Water)));
    assert!(items.iter().any(|i| i.kind == Kind::Glyph(Glyph::Stand)));
    assert!(
        items
            .iter()
            .any(|i| matches!(&i.kind, Kind::Text { text, .. } if text == "Stand/Move pending"))
    );
}

#[test]
fn today_reads_taken_and_skipped_with_strong_numbers() {
    let items = items(&state("2026-11-03T10:00:00", None));
    let today = items.iter().find(|i| i.id == TODAY).unwrap();
    let Kind::Spans { spans, right } = &today.kind else {
        panic!("{today:?}");
    };
    assert!(right);
    let words: String = spans.iter().map(|s| s.text.as_str()).collect();
    assert_eq!(words, "7 taken  2 skipped");
    assert_eq!(spans[0].style, Style::BodyStrong);
    assert_eq!(spans[2].style, Style::BodyStrong);
}

#[test]
fn only_the_countdown_changing_keeps_the_layout() {
    let item = switch_item(&week(), None, at("2026-11-03T10:00:00"));
    let p = prompt(Nudge::Pause, false, true);
    let a = content(&item, Some(&p), Duration::from_secs(90), (0, 0));
    let b = content(&item, Some(&p), Duration::from_secs(89), (0, 0));
    assert!(a.same_but_countdown(&b));
    let c = content(&item, Some(&p), Duration::from_secs(89), (1, 0));
    assert!(!a.same_but_countdown(&c));
    let d = content(&item, None, Duration::ZERO, (0, 0));
    assert!(!a.same_but_countdown(&d));
}

// ---- Layout ----

/// Every character 7px wide at 96 DPI, scaled with the DPI, at any style.
struct Fixed(u32);

impl Measure for Fixed {
    fn width(&self, _: Style, text: &str) -> i32 {
        px(7 * text.chars().count() as i32, self.0)
    }
}

fn laid_for_real(items: Vec<Item>, dpi: u32) -> Form {
    let fonts = Fonts::new(dpi);
    let mut form = Form::new(items);
    // SAFETY: the screen DC is released after use.
    unsafe {
        let dc = GetDC(None);
        form.lay_out(dpi, &GdiMeasure { dc, fonts: &fonts });
        ReleaseDC(None, dc);
    }
    form
}

/// The contents to try: on, pending with Water and Snooze, off, quiet.
fn samples() -> Vec<(&'static str, Content)> {
    let item = switch_item(&week(), None, at("2026-11-03T10:00:00"));
    let pending = content(
        &item,
        Some(&prompt(Nudge::Pause, true, true)),
        Duration::from_secs(119),
        (7, 2),
    );
    vec![
        ("on", state("2026-11-03T10:00:00", None)),
        ("pending", pending),
        (
            "off",
            state(
                "2026-11-03T10:00:00",
                Some(Switch::Off {
                    until: at("2026-11-04T08:00:00"),
                }),
            ),
        ),
        ("quiet", state("2026-11-05T18:00:00", None)),
    ]
}

fn overlap(a: &RECT, b: &RECT) -> bool {
    a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom
}

/// Every text and control stays inside the side margins (the footer buttons inside the band)
/// and clear of the others, with the real fonts, at every scale.
#[test]
fn items_keep_apart_inside_the_margins_at_every_scale() {
    for dpi in SCALES {
        for (name, c) in samples() {
            let form = laid_for_real(items(&c), dpi);
            let w = px(SIZE_FLYOUT, dpi);
            let pad = px(SPACE_4, dpi);
            let height = form.height(0);
            let parts: Vec<(usize, RECT)> = form
                .items()
                .iter()
                .zip(form.laid())
                .enumerate()
                .filter(|(_, (item, _))| {
                    !matches!(item.kind, Kind::Card { .. } | Kind::Band | Kind::Rule)
                })
                .map(|(i, (_, laid))| (i, laid.rect))
                .collect();
            for &(i, r) in &parts {
                let item = &form.items()[i];
                let footer = matches!(item.id, SETTINGS | QUIT);
                let margin = if footer { px(BAND_PAD, dpi) } else { pad };
                assert!(
                    r.left >= margin && r.right <= w - margin && r.bottom <= height,
                    "{name} at {dpi}: {item:?} at {r:?}"
                );
                for &(j, s) in &parts {
                    assert!(
                        i == j || !overlap(&r, &s),
                        "{name} at {dpi}: {:?} and {:?} overlap",
                        item.kind,
                        form.items()[j].kind
                    );
                }
            }
            // The pending card holds its content with `space-3` round it.
            if let Some(card) = form
                .items()
                .iter()
                .position(|i| matches!(i.kind, Kind::Card { .. }))
            {
                let cr = form.laid()[card].rect;
                let inner = px(SPACE_3, dpi);
                for id in [COUNTDOWN, SKIP, SNOOZE] {
                    let r = form.laid()[form.index(id).unwrap()].rect;
                    assert!(r.left >= cr.left + inner && r.right <= cr.right - inner);
                    assert!(r.top >= cr.top + inner && r.bottom <= cr.bottom - inner);
                }
            }
        }
    }
}

#[test]
fn the_flyout_is_320_wide_and_as_tall_as_its_rows_at_100_percent() {
    let mut on = Form::new(items(&samples()[0].1));
    on.lay_out(96, &Fixed(96));
    // Header 70, hairline, Nudges row 62, hairline, Today row 44, band 48.
    assert_eq!(on.height(0), 70 + 1 + 62 + 1 + 44 + 48);
    let mut pending = Form::new(items(&samples()[1].1));
    pending.lay_out(96, &Fixed(96));
    // The card: 12, title row 20, 4, countdown 18, 8, Water 20, 12, buttons 32, 12; then 12.
    let card = 12 + 20 + 4 + 18 + 8 + 20 + 12 + 32 + 12;
    assert_eq!(pending.height(0), on.height(0) + card + 12);
    // The Today count ends at the right margin.
    let today = pending.laid()[pending.index(TODAY).unwrap()].rect;
    assert_eq!(today.right, SIZE_FLYOUT - SPACE_4);
    assert_eq!(
        today.right - today.left,
        7 * "7 taken  2 skipped".len() as i32
    );
}

/// A note too long for one line wraps, and what is under it moves down.
#[test]
fn a_long_note_wraps_and_pushes_the_rows_down() {
    let mut c = samples()[0].1.clone();
    let mut short = Form::new(items(&c));
    short.lay_out(96, &Fixed(96));
    c.note = "Turn off until Wednesday 08:00, which is a long way off.".into();
    let mut long = Form::new(items(&c));
    long.lay_out(96, &Fixed(96));
    let lines = long.laid()[long.index(NOTE).unwrap()].lines.len() as i32;
    assert!(lines > 1);
    let grow = (lines - 1) * Style::Caption.line_height();
    assert_eq!(long.height(0), short.height(0) + grow);
    let quit = |f: &Form| f.laid()[f.index(QUIT).unwrap()].rect.top;
    assert_eq!(quit(&long), quit(&short) + grow);
}

// ---- Placement ----

/// A work area and taskbar for each edge, on a 1000 by 700 screen that need not be the real
/// one, with the tray icon where Windows puts it on that edge.
fn screen(edge: Edge) -> (RECT, RECT, RECT) {
    let r = |l, t, rr, b| RECT {
        left: l,
        top: t,
        right: rr,
        bottom: b,
    };
    match edge {
        Edge::Bottom => (
            r(0, 0, 1000, 652),
            r(0, 652, 1000, 700),
            r(900, 664, 924, 688),
        ),
        Edge::Top => (r(0, 48, 1000, 700), r(0, 0, 1000, 48), r(900, 12, 924, 36)),
        Edge::Left => (r(60, 0, 1000, 700), r(0, 0, 60, 700), r(18, 600, 42, 624)),
        Edge::Right => (
            r(0, 0, 940, 700),
            r(940, 0, 1000, 700),
            r(958, 600, 982, 624),
        ),
    }
}

#[test]
fn it_opens_next_to_the_icon_for_every_taskbar_edge() {
    let size = (320, 226);
    let gap = 12;
    for edge in [Edge::Bottom, Edge::Top, Edge::Left, Edge::Right] {
        let (work, bar, icon) = screen(edge);
        let p = place(size, icon, Some((edge, bar)), work, gap);
        let r = RECT {
            left: p.x,
            top: p.y,
            right: p.x + size.0,
            bottom: p.y + size.1,
        };
        assert!(
            r.left >= work.left && r.right <= work.right,
            "{edge:?}: {r:?} in {work:?}"
        );
        assert!(r.top >= work.top && r.bottom <= work.bottom, "{edge:?}");
        match edge {
            Edge::Bottom => {
                assert_eq!(r.bottom, bar.top - gap);
                assert_eq!(r.right, icon.right);
            }
            Edge::Top => {
                assert_eq!(r.top, bar.bottom + gap);
                assert_eq!(r.right, icon.right);
            }
            Edge::Left => {
                assert_eq!(r.left, bar.right + gap);
                assert_eq!(r.bottom, icon.bottom);
            }
            Edge::Right => {
                assert_eq!(r.right, bar.left - gap);
                assert_eq!(r.bottom, icon.bottom);
            }
        }
    }
}

#[test]
fn it_stays_inside_the_work_area() {
    let gap = 12;
    // An icon at the far right of a bottom taskbar: lined up, but kept `gap` in.
    let (work, bar, _) = screen(Edge::Bottom);
    let icon = RECT {
        left: 976,
        top: 664,
        right: 1000,
        bottom: 688,
    };
    let p = place((320, 226), icon, Some((Edge::Bottom, bar)), work, gap);
    assert_eq!(p.x, work.right - gap - 320);
    // An icon low on a side taskbar, a flyout taller than the room under it.
    let (work, bar, _) = screen(Edge::Left);
    let icon = RECT {
        left: 18,
        top: 670,
        right: 42,
        bottom: 694,
    };
    let p = place((320, 376), icon, Some((Edge::Left, bar)), work, gap);
    assert_eq!(p.y + 376, work.bottom - gap);
    // A flyout taller than the work area keeps its top-left inside it.
    let p = place((320, 900), icon, Some((Edge::Left, bar)), work, gap);
    assert_eq!(p.y, work.top + gap);
    // A monitor that does not start at 0, left of the primary one.
    let work = RECT {
        left: -1280,
        top: 0,
        right: 0,
        bottom: 984,
    };
    let bar = RECT {
        left: -1280,
        top: 984,
        right: 0,
        bottom: 1024,
    };
    let icon = RECT {
        left: -1270,
        top: 992,
        right: -1246,
        bottom: 1016,
    };
    let p = place((320, 226), icon, Some((Edge::Bottom, bar)), work, gap);
    assert_eq!((p.x, p.y), (-1280 + gap, 984 - gap - 226));
}

#[test]
fn an_icon_in_the_overflow_keeps_the_flyout_clear_of_it() {
    let (work, bar, _) = screen(Edge::Bottom);
    // The overflow window sits over the taskbar, so its icons are above it.
    let icon = RECT {
        left: 860,
        top: 600,
        right: 884,
        bottom: 624,
    };
    let p = place((320, 226), icon, Some((Edge::Bottom, bar)), work, 12);
    assert_eq!(p.y + 226, icon.top - 12);
    // With no taskbar known, it goes above the icon.
    let p = place((320, 226), icon, None, work, 12);
    assert_eq!(p.y + 226, icon.top - 12);
    assert_eq!(p.x + 320, icon.right);
}

// ---- Screen readers ----

const TREE: &str = r#"window
  text "breakpoint"
  text "On until 17:00"
  text "Nudges"
  switch "Nudges" on focused -- "Turn off until tomorrow 08:00."
  text "Turn off until tomorrow 08:00."
  group "Pause pending"
    text "Pause pending" h2
    text "Away 1:59 more and it counts"
    text "Drink some water."
    button "Snooze 5 min"
    button "Skip"
  text "Today"
  text "7 taken 2 skipped"
  button "Settings…"
  button "Quit"
"#;

/// The window is named "breakpoint" by its caption; the tree names the switch "Nudges" and
/// gives it its note, and the pending card is a group named by its title.
#[test]
fn the_flyout_reads_well_to_a_screen_reader_at_every_scale() {
    for dpi in SCALES {
        let mut form = laid_for_real(items(&samples()[1].1), dpi);
        form.focus_in();
        assert_eq!(Tree::new(&form, None).outline(), TREE, "at {dpi} DPI");
    }
}

// ---- The window ----

fn send_key(hwnd: HWND, vk: u16) {
    // SAFETY: a plain message to a live window.
    unsafe {
        SendMessageW(hwnd, WM_KEYDOWN, Some(WPARAM(vk.into())), Some(LPARAM(0)));
    }
}

/// An icon at the top-left of the primary monitor's work area, wherever that is.
fn some_icon() -> RECT {
    let work = work_area(&RECT::default());
    RECT {
        left: work.left + 40,
        top: work.top + 40,
        right: work.left + 64,
        bottom: work.top + 64,
    }
}

fn drain() -> Vec<Command> {
    std::iter::from_fn(tray::take).collect()
}

#[test]
fn the_window_is_a_borderless_popup_that_esc_and_lost_activation_close() {
    drain();
    let c = samples()[0].1.clone();
    let handle = make(&c, some_icon()).unwrap();
    let hwnd = handle.hwnd();
    // SAFETY: plain queries on a live window.
    let (style, ex, client) = unsafe {
        let mut client = RECT::default();
        GetClientRect(hwnd, &mut client).unwrap();
        (
            GetWindowLongW(hwnd, GWL_STYLE) as u32,
            GetWindowLongW(hwnd, GWL_EXSTYLE) as u32,
            client,
        )
    };
    assert_ne!(style & WS_POPUP.0, 0);
    assert_eq!(style & WS_CAPTION.0, 0, "no caption");
    assert_ne!(ex & WS_EX_TOOLWINDOW.0, 0, "off the taskbar");
    assert_ne!(ex & WS_EX_TOPMOST.0, 0);
    let r = handle.window_rect();
    assert_eq!(
        (r.right - r.left, r.bottom - r.top),
        (client.right, client.bottom),
        "no frame"
    );
    // SAFETY: no pointers.
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    assert_eq!(client.right, px(SIZE_FLYOUT, dpi));
    let work = work_area(&r);
    assert!(r.left >= work.left && r.right <= work.right);
    assert!(r.top >= work.top && r.bottom <= work.bottom);
    assert!(is_open());

    LOST.set(None);
    send_key(hwnd, VK_ESCAPE.0);
    assert!(!is_open(), "Esc closes it");
    assert!(
        !lost_at(&EVERYWHERE),
        "after Esc, a click on the icon opens it again at once"
    );
    assert_eq!(drain(), []);

    let handle = make(&c, some_icon()).unwrap();
    // SAFETY: as above; the posted close arrives on the pump.
    unsafe {
        SendMessageW(
            handle.hwnd(),
            WM_ACTIVATE,
            Some(WPARAM(WA_INACTIVE as usize)),
            Some(LPARAM(0)),
        );
    }
    pump(50);
    assert!(!is_open(), "losing activation closes it");
    assert!(
        lost_at(&EVERYWHERE),
        "a press on the icon where the pointer was is the one that closed it"
    );
    assert_eq!(drain(), []);
    LOST.set(None);
}

/// A rectangle round every screen.
const EVERYWHERE: RECT = RECT {
    left: -100_000,
    top: -100_000,
    right: 100_000,
    bottom: 100_000,
};

/// Only a press on the icon itself, soon after a click outside closed the flyout, is the
/// press that closed it; a later one, or one elsewhere, is a click of its own.
#[test]
fn a_lost_close_counts_only_for_a_press_on_the_icon_soon_after() {
    let icon = RECT {
        left: 900,
        top: 740,
        right: 924,
        bottom: 764,
    };
    let on_icon = POINT { x: 910, y: 750 };
    LOST.set(Some((Instant::now(), on_icon)));
    assert!(lost_at(&icon));
    LOST.set(Some((Instant::now(), POINT { x: 500, y: 300 })));
    assert!(!lost_at(&icon), "the click was elsewhere");
    let long_ago = Instant::now() - Duration::from_secs(5);
    LOST.set(Some((long_ago, on_icon)));
    assert!(!lost_at(&icon), "too long ago");
    LOST.set(None);
    assert!(!lost_at(&icon));
}

#[test]
fn the_watch_closes_on_a_press_outside_only() {
    let flyout = RECT {
        left: 600,
        top: 300,
        right: 920,
        bottom: 526,
    };
    assert!(outside_press(true, POINT { x: 910, y: 750 }, &flyout));
    assert!(!outside_press(true, POINT { x: 700, y: 400 }, &flyout));
    assert!(!outside_press(false, POINT { x: 910, y: 750 }, &flyout));
}

/// A flyout that could not take the foreground is watched until it closes.
#[test]
fn the_watch_runs_while_the_flyout_is_open_and_stops_with_it() {
    let c = samples()[0].1.clone();
    make(&c, some_icon()).unwrap();
    watch();
    assert_ne!(WATCH.get(), 0);
    // A tick with no button down leaves it open.
    pump(80);
    if unsafe { GetAsyncKeyState(VK_LBUTTON.0.into()) } >= 0 {
        assert!(is_open());
    }
    close();
    assert_eq!(WATCH.get(), 0);
    assert!(!is_open());
}

/// The icon opens the flyout with the app's latest content, and nothing before there is any.
#[test]
fn the_latest_content_is_what_a_click_opens_with() {
    LATEST.set(None);
    open(some_icon(), false);
    assert!(!is_open(), "no content yet");
    let c = samples()[0].1.clone();
    show_content(c.clone());
    assert_eq!(LATEST.with_borrow(Clone::clone), Some(c));
    assert!(!is_open(), "handing content opens nothing");
    LATEST.set(None);
}

#[test]
fn every_action_closes_the_flyout_and_queues_its_command() {
    drain();
    let item = switch_item(&week(), None, at("2026-11-03T10:00:00"));
    let c = content(
        &item,
        Some(&prompt(Nudge::Pause, false, true)),
        Duration::from_secs(60),
        (0, 0),
    );
    // Space and Enter on the switch, then Tab to each button and Enter.
    let cases: [(&[u16], Command); 6] = [
        (&[VK_SPACE.0], Command::Switch),
        (&[VK_RETURN.0], Command::Switch),
        (&[VK_TAB.0, VK_RETURN.0], Command::Snooze),
        (&[VK_TAB.0, VK_TAB.0, VK_SPACE.0], Command::Skip),
        (
            &[VK_TAB.0, VK_TAB.0, VK_TAB.0, VK_RETURN.0],
            Command::Settings,
        ),
        (
            &[VK_TAB.0, VK_TAB.0, VK_TAB.0, VK_TAB.0, VK_RETURN.0],
            Command::Quit,
        ),
    ];
    for (keys, command) in cases {
        let handle = make(&c, some_icon()).unwrap();
        for &vk in keys {
            send_key(handle.hwnd(), vk);
        }
        assert!(!is_open(), "{command:?} closes it");
        assert_eq!(drain(), [command]);
    }
}

#[test]
fn a_change_while_open_lays_it_out_again_next_to_its_icon() {
    let item = switch_item(&week(), None, at("2026-11-03T10:00:00"));
    let plain = content(&item, None, Duration::ZERO, (0, 0));
    let handle = make(&plain, some_icon()).unwrap();
    let before = handle.window_rect();
    let p = prompt(Nudge::Pause, true, true);
    let pending = content(&item, Some(&p), Duration::from_secs(90), (0, 0));
    update(&pending);
    let after = handle.window_rect();
    assert!(after.bottom - after.top > before.bottom - before.top);
    assert_eq!(after.right, before.right, "still lined up with the icon");
    assert!(handle.screen_rect(SNOOZE).is_some());
    // The countdown ticks in place.
    let tick = content(&item, Some(&p), Duration::from_secs(89), (0, 0));
    update(&tick);
    assert_eq!(handle.window_rect(), after);
    close();
    assert!(!is_open());
}

// ---- Shots ----

/// For a look by eye: `BP_SHOTS=<folder> cargo test flyout::tests::shots -- --ignored` writes
/// the flyout on, pending (with Water), off and quiet to BMPs there: light, dark and high
/// contrast (the High Contrast Black colours) at 100, 150 and 200 %. The first has the focus
/// ring on the switch, as when opened from the keyboard.
#[test]
#[ignore = "writes BMPs"]
fn shots() {
    let Ok(folder) = std::env::var("BP_SHOTS") else {
        eprintln!("set BP_SHOTS to a folder for the shots");
        return;
    };
    // SAFETY: no pointers.
    unsafe {
        SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    crate::platform::paint::startup();
    let schemes = [
        ("light", Colours::of(Theme::Light)),
        ("dark", Colours::of(Theme::Dark)),
        ("contrast", Colours::high_contrast(high_contrast_black)),
    ];
    for (scheme, c) in schemes {
        for dpi in SCALES {
            for (k, (name, content)) in samples().into_iter().enumerate() {
                let fonts = Fonts::new(dpi);
                let mut form = laid_for_real(items(&content), dpi);
                if k == 0 {
                    form.focus(SWITCH);
                    form.focus_in();
                }
                let (w, h) = (px(SIZE_FLYOUT, dpi), form.height(0));
                let path = Path::new(&folder).join(format!("flyout-{name}-{scheme}-{dpi}.bmp"));
                render(w, h, &path, |dc| unsafe {
                    let looks = form.looks();
                    paint_on(dc, w, h, c.raised, &fonts, dpi, |canvas, m, runs| {
                        paint_form(canvas, &form, &looks, &c, m, runs);
                    });
                });
            }
        }
    }
}

/// For a look by eye: `BP_SHOTS=<folder> cargo test flyout::tests::live -- --ignored` opens the
/// real flyout, pending, next to the right end of the taskbar in the current theme and scale,
/// and copies that part of the screen, round corners and shadow included, to `flyout-live.bmp`.
#[test]
#[ignore = "shows the flyout and copies the screen"]
fn live() {
    use windows::Win32::Graphics::Gdi::SRCCOPY;
    use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

    let Ok(folder) = std::env::var("BP_SHOTS") else {
        eprintln!("set BP_SHOTS to a folder for the shots");
        return;
    };
    // SAFETY: no pointers.
    unsafe {
        SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    let (edge, bar) = taskbar().expect("a taskbar");
    // Where the notification area is on that edge, roughly.
    let icon = match edge {
        Edge::Bottom | Edge::Top => RECT {
            left: bar.right - 120,
            top: bar.top + 8,
            right: bar.right - 96,
            bottom: bar.top + 32,
        },
        Edge::Left | Edge::Right => RECT {
            left: bar.left + 8,
            top: bar.bottom - 120,
            right: bar.left + 32,
            bottom: bar.bottom - 96,
        },
    };
    show_content(samples()[1].1.clone());
    open(icon, true);
    pump(400);
    let handle = OPEN.get().unwrap();
    let r = handle.window_rect();
    // SAFETY: no pointers.
    let foreground = unsafe { GetForegroundWindow() } == handle.hwnd();
    eprintln!("taskbar {edge:?} {bar:?}, flyout {r:?}, foreground {foreground}");
    let m = 24;
    let (w, h) = (r.right - r.left + 2 * m, r.bottom - r.top + 2 * m);
    render(
        w,
        h,
        &Path::new(&folder).join("flyout-live.bmp"),
        |dc| unsafe {
            let screen = GetDC(None);
            let _ = windows::Win32::Graphics::Gdi::BitBlt(
                dc,
                0,
                0,
                w,
                h,
                Some(screen),
                r.left - m,
                r.top - m,
                SRCCOPY,
            );
            ReleaseDC(None, screen);
        },
    );
    close();
}
