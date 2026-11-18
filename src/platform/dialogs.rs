//! The first-run wizard (SPEC §9.1), painted with the form kit in the card layout of DESIGN.md
//! §6, and the message ids the app's window takes from the dialogs.
//!
//! A modeless kit window, at most one at a time. It shows the lines the core writes and reads
//! back the [`Picks`]: the profile, from three selectable cards with a day timeline each, and
//! Start with Windows. However it closes, it posts [`WM_WIZARD_DONE`], and the owner collects
//! how with [`take_wizard`]: Start, Esc and × accept the picks; Change… hands them on to the
//! settings window, opened where the wizard's top-left was.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use breakpoint::core::config::Profile;
use breakpoint::core::wizard::{self, Picks};
use windows::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    IsIconic, MB_ICONWARNING, MB_OK, MessageBoxW, PostMessageW, SW_RESTORE, SetForegroundWindow,
    ShowWindow, WM_APP,
};
use windows::core::{HSTRING, w};

use super::kit::window::{self, Frame, Handle, Spec};
use super::kit::{Aside, ButtonKind, Event, Id, Ink, Item, Kind, Place, Value};
use super::paint::Box2;
use super::settings_window;
use super::theme::{
    CHECKBOX, CHOICE_PAD_Y, Colours, SIZE_BUTTON, SPACE_1, SPACE_2, SPACE_3, SPACE_4, SPACE_6,
    SPACE_8, Style,
};

/// Asks the main window to open Settings, from a second launch (SPEC §11).
pub const WM_OPEN_SETTINGS: u32 = WM_APP + 3;
/// Posted to the owner when the wizard closed; see [`take_wizard`].
pub const WM_WIZARD_DONE: u32 = WM_APP + 6;

/// How the wizard closed.
pub enum WizardDone {
    /// Start, × or Esc: accept the picks.
    Start(Picks),
    /// Change…: open the settings window with the picks, its top-left at this screen point,
    /// where the wizard's was.
    Change(Picks, POINT),
}

thread_local! {
    /// The wizard while it is open.
    static WIZARD: Cell<Option<Handle>> = const { Cell::new(None) };
    /// Where the wizard's end is posted.
    static OWNER: Cell<Option<HWND>> = const { Cell::new(None) };
    static WIZARD_DONE: RefCell<Option<WizardDone>> = const { RefCell::new(None) };
}

// ---- Item ids ----

/// The heading, texts, the card and the band: nothing reads or changes them.
const FIXED: Id = 0;
/// The three profile cards in `Profile::ALL` order: `PROFILE` to `PROFILE + 2`.
const PROFILE: Id = 10;
const CHANGE: Id = 20;
const START_WITH_WINDOWS: Id = 30;
const START: Id = 40;
/// The profile cards' radio group.
const PROFILES: u16 = 1;

// ---- Layout, at 96 DPI ----

/// Client width: the settings window's, so Change… opens it where the wizard was.
const WIDTH: i32 = settings_window::WIDTH;
/// Window padding, and the gap between sections.
const PAD: i32 = SPACE_6;
/// Card content runs `space-4` in from the card's sides.
const LEFT: i32 = PAD + SPACE_4;
const RIGHT: i32 = WIDTH - PAD - SPACE_4;
const LINE: i32 = Style::Body.line_height();
const NOTE: i32 = Style::Caption.line_height();
/// A profile card: its padding, the name and two lines of numbers.
const CHOICE_H: i32 = 2 * CHOICE_PAD_Y + LINE + 2 * NOTE;
/// The Work week card's one row: a label and the line under it.
const ROW_NOTED: i32 = 2 * SPACE_3 + LINE + NOTE;
const BUTTON_W: i32 = 88;
const BAND: i32 = SIZE_BUTTON + 2 * SPACE_4;
/// The day timeline at the right of each profile card.
const TIMELINE_W: i32 = 150;
const TIMELINE_H: i32 = 24;
/// The timeline's baseline, and its ticks.
const BASELINE: f32 = 2.0;
const TICK_W: f32 = 3.0;
const STAND_TICK: f32 = 12.0;
const PAUSE_TICK: f32 = 20.0;

// ---- The day timeline ----

/// A nudge on a profile's day timeline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tick {
    Stand,
    Pause,
}

/// When `profile`'s Stand/Move and Pause fall due across a work day `span` minutes long, in
/// active minutes from its start, in order. A Pause takes the place of a Stand/Move that falls
/// at the same minute, as it absorbs it (SPEC §5.3).
fn ticks(profile: Profile, span: u32) -> Vec<(u32, Tick)> {
    let nudges = profile.nudges();
    let due = |every: u32| (1..).map(move |k| k * every).take_while(move |&m| m < span);
    let pause = nudges.pause.every_minutes;
    let mut all: Vec<_> = due(pause)
        .map(|m| (m, Tick::Pause))
        .chain(
            due(nudges.stand.every_minutes)
                .filter(|m| m % pause != 0)
                .map(|m| (m, Tick::Stand)),
        )
        .collect();
    all.sort_by_key(|&(m, _)| m);
    all
}

/// The baseline and each tick of `profile`'s timeline in the box `b`, at `dpi`: the 2px
/// baseline across `b`'s middle, then each tick 3 wide, centred on its minute, `stand` 12 tall
/// and `pause` 20 tall, centred on the baseline.
fn timeline(profile: Profile, span: u32, b: Box2, dpi: u32) -> (Box2, Vec<(Box2, Tick)>) {
    let s = |v: f32| v * dpi as f32 / 96.0;
    let mid = b.y + b.h / 2.0;
    let base = Box2 {
        y: mid - s(BASELINE) / 2.0,
        h: s(BASELINE),
        ..b
    };
    let ticks = ticks(profile, span)
        .into_iter()
        .map(|(minute, tick)| {
            let tall = s(match tick {
                Tick::Stand => STAND_TICK,
                Tick::Pause => PAUSE_TICK,
            });
            let x = b.x + b.w * minute as f32 / span as f32;
            let at = Box2 {
                x: x - s(TICK_W) / 2.0,
                y: mid - tall / 2.0,
                w: s(TICK_W),
                h: tall,
            };
            (at, tick)
        })
        .collect();
    (base, ticks)
}

/// The paint hook for `profile`'s timeline over a work day `span` minutes long.
fn timeline_aside(profile: Profile, span: u32) -> Aside {
    Aside {
        w: TIMELINE_W,
        h: TIMELINE_H,
        paint: Rc::new(move |canvas, b, c: &Colours, dpi| {
            let (base, ticks) = timeline(profile, span, b, dpi);
            canvas.fill_rect(c.hairline, base);
            for (at, tick) in ticks {
                let colour = match tick {
                    Tick::Stand => c.stand,
                    Tick::Pause => c.pause,
                };
                canvas.fill_round_rect(colour, at, at.w / 2.0);
            }
        }),
    }
}

// ---- The window ----

/// A profile's name, the title of its card.
pub(super) fn profile_name(profile: Profile) -> &'static str {
    match profile {
        Profile::Gentle => "Gentle",
        Profile::Balanced => "Balanced",
        Profile::Strict => "Strict",
    }
}

/// The core's line of a profile's numbers without the "Name: " in front, which the card's
/// title already shows, and so with a capital first letter.
pub(super) fn numbers_line(profile: Profile) -> String {
    let line = wizard::profile_line(profile);
    let numbers = match line.split_once(": ") {
        Some((name, numbers)) if name == profile_name(profile) => numbers,
        _ => &line,
    };
    let mut chars = numbers.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// A card's numbers: the cadences on the first line, Water on the second.
fn card_note(profile: Profile) -> String {
    numbers_line(profile).replacen(". ", ".\n", 1)
}

/// The whole wizard as kit items, filled with `picks`, top to bottom in reading order.
pub(super) fn items(picks: &Picks) -> Vec<Item> {
    let mut out = Vec::new();
    let mut add = |id, at, kind| out.push(Item::new(id, at, kind));
    let config = picks.config();
    let hours = config.work_hours;
    let span = hours
        .end
        .minutes()
        .saturating_sub(hours.start.minutes())
        .max(1);

    let mut y = PAD;
    add(
        FIXED,
        Place::at(PAD, y),
        Kind::text("Pick a rhythm", Style::TitleLg, Ink::Main),
    );
    y += Style::TitleLg.line_height() + SPACE_1;
    add(
        FIXED,
        Place::at(PAD, y),
        Kind::text(
            "You can change any of it later in settings.",
            Style::Body,
            Ink::Muted,
        ),
    );
    // `space-8` from the heading to the first section.
    y += LINE + SPACE_8;

    // The profiles: one selectable card each, one Tab stop.
    for (i, profile) in (0..).zip(Profile::ALL) {
        if i > 0 {
            y += SPACE_2;
        }
        add(
            PROFILE + i,
            // Two lines of numbers reserved; a third at some scale grows the card.
            Place::new(PAD, y, WIDTH - 2 * PAD, CHOICE_H),
            Kind::Choice {
                group: PROFILES,
                title: profile_name(profile).into(),
                note: card_note(profile),
                selected: profile == picks.profile,
                aside: Some(timeline_aside(profile, span)),
            },
        );
        y += CHOICE_H;
    }

    // The work week, read-only, with Change….
    y += PAD;
    add(
        FIXED,
        Place::new(PAD, y, WIDTH - 2 * PAD, ROW_NOTED),
        Kind::card(Vec::new()),
    );
    let line = y + SPACE_3;
    add(
        FIXED,
        Place::at(LEFT, line),
        Kind::text("Work week", Style::BodyStrong, Ink::Main),
    );
    add(
        FIXED,
        Place::at(LEFT, line + LINE),
        Kind::text(&wizard::work_week_line(&config), Style::Caption, Ink::Muted),
    );
    add(
        CHANGE,
        Place::new(
            RIGHT - BUTTON_W,
            y + (ROW_NOTED - SIZE_BUTTON) / 2,
            BUTTON_W,
            0,
        ),
        Kind::button("Change\u{2026}", ButtonKind::Secondary),
    );
    y += ROW_NOTED + PAD;

    // Start with Windows, and the note on the tray under its label.
    add(
        START_WITH_WINDOWS,
        Place::at(PAD, y),
        Kind::Checkbox {
            on: picks.start_with_windows,
            label: "Start with Windows".into(),
        },
    );
    y += LINE;
    add(
        FIXED,
        Place::at(PAD + CHECKBOX + SPACE_2, y),
        Kind::text(
            "breakpoint lives in the tray, under ^ if you don't see it.",
            Style::Caption,
            Ink::Muted,
        ),
    );
    y += NOTE + PAD;

    // The footer band: Start, the default, which Esc and × press too (SPEC §9.1).
    add(FIXED, Place::new(0, y, WIDTH, BAND), Kind::Band);
    add(
        START,
        Place::new(WIDTH - PAD - BUTTON_W, y + SPACE_4, BUTTON_W, 0),
        Kind::Button {
            label: "Start".into(),
            kind: ButtonKind::Primary,
            default: true,
            cancel: true,
            icon: None,
        },
    );
    out
}

/// Opens the first-run wizard with `picks` preselected (SPEC §9.1), or brings it to the front.
/// `owner` receives [`WM_WIZARD_DONE`] when it closes. The focus starts on the picked profile.
pub fn open_wizard(owner: HWND, picks: Picks) {
    if let Some(handle) = current() {
        bring_to_front(handle);
        return;
    }
    OWNER.set(Some(owner));
    if let Some(handle) = create(&picks, None) {
        handle.show();
    }
}

/// Makes the wizard, hidden, centred unless `at` is given.
fn create(picks: &Picks, at: Option<POINT>) -> Option<Handle> {
    let spec = Spec {
        caption: "breakpoint".into(),
        width: WIDTH,
        height: 0,
        items: items(picks),
        at,
        // No owner, so the window has its own taskbar button.
        owner: None,
        frame: Frame::Dialog,
    };
    let handle = window::open(spec, on_event).ok()?;
    WIZARD.set(Some(handle));
    Some(handle)
}

fn current() -> Option<Handle> {
    WIZARD.get().filter(|h| h.is_open())
}

/// How the wizard closed, once, after [`WM_WIZARD_DONE`].
pub fn take_wizard() -> Option<WizardDone> {
    WIZARD_DONE.take()
}

/// Brings the wizard, or else the settings window, to the front: a second launch during first
/// run (SPEC §11). False if neither is open.
pub fn to_front() -> bool {
    match current() {
        Some(handle) => {
            bring_to_front(handle);
            true
        }
        None => settings_window::to_front(),
    }
}

fn bring_to_front(handle: Handle) {
    // SAFETY: the window is live.
    unsafe {
        if IsIconic(handle.hwnd()).as_bool() {
            let _ = ShowWindow(handle.hwnd(), SW_RESTORE);
        }
        let _ = SetForegroundWindow(handle.hwnd());
    }
}

/// One message box with no owner, for what went wrong after a window closed.
pub fn error(text: &str) {
    // SAFETY: both strings live through the call.
    unsafe {
        MessageBoxW(
            None,
            &HSTRING::from(text),
            w!("breakpoint"),
            MB_OK | MB_ICONWARNING,
        );
    }
}

fn on_event(handle: Handle, event: Event) {
    match event {
        // Start, and Esc and ×, which press it: all accept what the page shows.
        Event::Pressed(START) | Event::Close => done(handle, false),
        Event::Pressed(CHANGE) => done(handle, true),
        _ => {}
    }
}

/// The picks the window shows.
fn read(handle: Handle) -> Picks {
    let profile = (0..)
        .zip(Profile::ALL)
        .find(|&(i, _)| handle.value(PROFILE + i) == Some(Value::On(true)))
        .map_or(Picks::default().profile, |(_, p)| p);
    Picks {
        profile,
        start_with_windows: handle.value(START_WITH_WINDOWS) == Some(Value::On(true)),
    }
}

/// The wizard closes, by Start, ×, Esc or Change…: hand the picks over and go.
fn done(handle: Handle, change: bool) {
    let picks = read(handle);
    let done = if change {
        let r = handle.window_rect();
        WizardDone::Change(
            picks,
            POINT {
                x: r.left,
                y: r.top,
            },
        )
    } else {
        WizardDone::Start(picks)
    };
    WIZARD_DONE.set(Some(done));
    if let Some(owner) = OWNER.get() {
        // SAFETY: `owner` is the app's main window, alive as long as the app.
        let _ = unsafe { PostMessageW(Some(owner), WM_WIZARD_DONE, WPARAM(0), LPARAM(0)) };
    }
    WIZARD.set(None);
    handle.close();
}

#[cfg(test)]
mod tests;
