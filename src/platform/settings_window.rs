//! The settings window (SPEC §9.2), painted with the form kit in the card layout of DESIGN.md §6.
//!
//! A modeless kit window, at most one at a time, that lets nudges keep firing while it is open.
//! It copies the core's [`Form`] into its controls and back; the core decides the profile shown,
//! what picking one fills in and whether a Save is valid. A valid Save closes the window and
//! posts [`WM_SETTINGS_SAVED`] to the owner, which collects it with [`take_saved`] and applies it
//! from its own window procedure. Cancel, Esc or × posts [`WM_SETTINGS_CANCELLED`] instead.
//!
//! The layout is data: [`items`] places the heading, the three cards, every control and the
//! footer band at 96 DPI from the tokens of [`super::theme`]. The kit scales it for the monitor,
//! paints it in the app theme (or high contrast), and runs the pointer and the keyboard. The
//! list order is the Tab order, which follows the reading order.

use std::cell::{Cell, RefCell};
use std::ops::RangeInclusive;

use breakpoint::core::config::{self, Enforcement, Profile};
use breakpoint::core::settings::{AwayRow, Field, Form, Settings};
use breakpoint::core::time::{TimeOfDay, Weekday};
use windows::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    IsIconic, MB_ICONWARNING, MB_OK, MessageBoxW, PostMessageW, SW_RESTORE, SetForegroundWindow,
    ShowWindow, WM_APP,
};
use windows::core::HSTRING;

use super::dialogs::{numbers_line, profile_name};
use super::kit::window::{self, Frame, Handle, Spec};
use super::kit::{ButtonKind, Entry, Event, Id, Ink, Item, Kind, Place, Value};
use super::paint::Glyph;
use super::theme::{
    PILL_W, SIZE_BUTTON, SIZE_FIELD, SIZE_GLYPH, SPACE_1, SPACE_2, SPACE_3, SPACE_4, SPACE_6,
    STEPPER_VALUE, STEPPER_ZONE, SWITCH_H, SWITCH_W, Style,
};

/// Posted to the owner when a valid Save closed the settings window; see [`take_saved`].
pub const WM_SETTINGS_SAVED: u32 = WM_APP + 4;
/// Posted to the owner when the settings window closed without a Save: Cancel, Esc or ×.
pub const WM_SETTINGS_CANCELLED: u32 = WM_APP + 5;

/// The window's caption, and the caption of the message box a refused Save shows.
const CAPTION: &str = "breakpoint settings";

/// What a valid Save hands the app.
pub struct Saved {
    pub settings: Settings,
    pub start_with_windows: bool,
}

thread_local! {
    /// The settings window while it is open.
    static OPEN: Cell<Option<Handle>> = const { Cell::new(None) };
    /// Where a Save or a cancel is posted.
    static OWNER: Cell<Option<HWND>> = const { Cell::new(None) };
    static SAVED: RefCell<Option<Saved>> = const { RefCell::new(None) };
}

// ---- Item ids ----

/// Texts, cards, glyphs and the band: nothing reads or changes them.
const FIXED: Id = 0;
/// The seven work-day pills, Sunday first: `DAY` to `DAY + 6`.
const DAY: Id = 10;
const FROM: Id = 20;
const TO: Id = 21;
const PROFILE: Id = 30;
const STAND: Row = Row {
    on: 40,
    every: 41,
    length: 42,
    level: 43,
};
const PAUSE: Row = Row {
    on: 50,
    every: 51,
    length: 52,
    level: 53,
};
const WATER_ON: Id = 60;
const WATER_EVERY: Id = 61;
const START_WITH_WINDOWS: Id = 70;
const SAVE: Id = 80;
const CANCEL: Id = 81;

/// The controls of the Stand up and move or the Pause row.
struct Row {
    on: Id,
    every: Id,
    length: Id,
    level: Id,
}

// ---- Layout, at 96 DPI ----

/// Client width. The wizard has the same, so Change… opens this window where the wizard was.
pub const WIDTH: i32 = 600;
/// Window padding, and the gap between sections.
const PAD: i32 = SPACE_6;
/// Card content runs `space-4` in from the card's sides.
const LEFT: i32 = PAD + SPACE_4;
const RIGHT: i32 = WIDTH - PAD - SPACE_4;
/// The label column of the Work week card.
const LABEL: i32 = 104;
/// Where the controls right of the labels start.
const FIELD: i32 = LEFT + LABEL + SPACE_3;
const LINE: i32 = Style::Body.line_height();
const NOTE: i32 = Style::Caption.line_height();
/// A stepper's width: its − and + zones and the value between them.
const STEPPER: i32 = 2 * STEPPER_ZONE + STEPPER_VALUE;
/// From one day pill to the next.
const DAY_STEP: i32 = PILL_W + SPACE_1;
/// Room for "to" between the two time steppers.
const TO_W: i32 = 16;
const DROP_W: i32 = 176;
/// The Soft / Firm toggle.
const TOGGLE_W: i32 = 112;
/// A nudge's name, and the line of controls under it, start after its glyph.
const NAME: i32 = LEFT + SIZE_GLYPH + SPACE_2;
/// Room for the words before a stepper in a nudge row, the gap to the stepper included.
const EVERY_W: i32 = 48;
const AWAY_W: i32 = 64;
const AT_MOST_W: i32 = 96;
/// The footer buttons.
const BUTTON_W: i32 = 88;
const BAND: i32 = SIZE_BUTTON + 2 * SPACE_4;

/// Card rows: `space-3` above and below the content.
/// A row of one control 32 tall, such as the day pills.
const ROW_FIELD: i32 = 2 * SPACE_3 + SIZE_FIELD;
/// A row of a label and a note under it: Profile, Start with Windows.
const ROW_NOTED: i32 = 2 * SPACE_3 + LINE + NOTE;
/// A nudge row: the glyph, name and switch, then its steppers.
const ROW_NUDGE: i32 = 2 * SPACE_3 + SWITCH_H + SPACE_2 + SIZE_FIELD;

const DAY_LABELS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const LEVELS: [&str; 2] = ["Soft", "Firm"];

/// Collects items top to bottom.
#[derive(Default)]
struct Builder(Vec<Item>);

impl Builder {
    fn add(&mut self, id: Id, at: Place, kind: Kind) {
        self.0.push(Item::new(id, at, kind));
    }

    fn text(&mut self, x: i32, y: i32, text: &str, style: Style, ink: Ink) {
        self.add(FIXED, Place::at(x, y), Kind::text(text, style, ink));
    }

    /// `body` text centred on a control 32 tall whose top is `y`.
    fn label(&mut self, x: i32, y: i32, text: &str) {
        self.text(x, y + (SIZE_FIELD - LINE) / 2, text, Style::Body, Ink::Main);
    }

    /// A section after `y`: `space-6`, the heading, `space-2`, then a card of rows `rows`
    /// tall. Moves `y` to the card's bottom and returns the card's top.
    fn section(&mut self, y: &mut i32, heading: &str, rows: &[i32]) -> i32 {
        *y += PAD;
        self.text(PAD, *y, heading, Style::BodyStrong, Ink::Main);
        let top = *y + LINE + SPACE_2;
        let lines = rows
            .iter()
            .scan(0, |at, row| {
                *at += row;
                Some(*at)
            })
            .collect::<Vec<_>>();
        let height = lines.last().copied().unwrap_or(0);
        let rows = lines[..lines.len().saturating_sub(1)].to_vec();
        self.add(
            FIXED,
            Place::new(PAD, top, WIDTH - 2 * PAD, height),
            Kind::card(rows),
        );
        *y = top + height;
        top
    }

    /// A nudge's first line in the row at `top`: glyph, name and switch. Returns the top of
    /// the line under it.
    fn nudge(&mut self, top: i32, glyph: Glyph, name: &str, on_id: Id, on: bool) -> i32 {
        let line = top + SPACE_3;
        self.add(
            FIXED,
            Place::at(LEFT, line + (SWITCH_H - SIZE_GLYPH) / 2),
            Kind::Glyph(glyph),
        );
        let name_y = line + (SWITCH_H - LINE) / 2;
        self.text(NAME, name_y, name, Style::BodyStrong, Ink::Main);
        self.add(
            on_id,
            Place::at(RIGHT - SWITCH_W, line),
            Kind::Switch { on },
        );
        line + SWITCH_H + SPACE_2
    }

    /// A Stand up and move or Pause row at `top`.
    fn away(
        &mut self,
        top: i32,
        glyph: Glyph,
        name: &str,
        row: &Row,
        values: &AwayRow,
        ranges: [&RangeInclusive<u32>; 2],
    ) {
        let line = self.nudge(top, glyph, name, row.on, values.on);
        self.label(NAME, line, "Every");
        let every = NAME + EVERY_W;
        self.add(
            row.every,
            Place::at(every, line),
            stepper(&values.every, ranges[0], CADENCE_STEP),
        );
        let away = every + STEPPER + SPACE_4;
        self.label(away, line, "Away for");
        self.add(
            row.length,
            Place::at(away + AWAY_W, line),
            stepper(&values.length, ranges[1], 1),
        );
        self.add(
            row.level,
            Place::new(RIGHT - TOGGLE_W, line, TOGGLE_W, 0),
            Kind::Segmented {
                options: LEVELS.map(String::from).to_vec(),
                selected: level_index(values.enforcement),
            },
        );
    }

    /// A label in `body` with a note in `caption` under it at the left of the row at `top`.
    fn noted(&mut self, top: i32, label: &str, note: &str) {
        let line = top + SPACE_3;
        self.text(LEFT, line, label, Style::Body, Ink::Main);
        self.text(LEFT, line + LINE, note, Style::Caption, Ink::Muted);
    }
}

/// A cadence stepper moves in 5 minutes, a length in 1; any whole number can be typed.
const CADENCE_STEP: i32 = 5;

fn stepper(value: &str, range: &RangeInclusive<u32>, step: i32) -> Kind {
    let (min, max) = (*range.start() as i32, *range.end() as i32);
    Kind::Stepper {
        value: value.trim().parse().unwrap_or(min),
        min,
        max,
        step,
        unit: "min".into(),
    }
}

fn level_index(level: Enforcement) -> usize {
    match level {
        Enforcement::Soft => 0,
        Enforcement::Firm => 1,
    }
}

/// The drop-down's index of the profile `form` matches; `None` shows Custom.
fn shown_profile(form: &Form) -> Option<usize> {
    let profile = form.profile()?;
    Profile::ALL.iter().position(|&p| p == profile)
}

/// A profile's line in the menu: its numbers without the Water sentence, which is the same for
/// every profile, so the menu stays narrow.
fn menu_note(profile: Profile) -> String {
    let line = numbers_line(profile);
    match line.split_once(". ") {
        Some((numbers, _)) => numbers.to_owned(),
        None => line,
    }
}

/// The whole window as kit items, filled with `form`, top to bottom in reading order.
pub(super) fn items(form: &Form, start_with_windows: bool) -> Vec<Item> {
    let mut b = Builder::default();
    let mut y = PAD;
    b.text(PAD, y, "Settings", Style::TitleLg, Ink::Main);
    y += Style::TitleLg.line_height() + SPACE_1;
    let lead = "Nudges fall due only inside your work week.";
    b.text(PAD, y, lead, Style::Body, Ink::Muted);
    y += LINE;

    // Work week: the day pills, then the hours.
    let top = b.section(&mut y, "Work week", &[ROW_FIELD, ROW_FIELD]);
    let line = top + SPACE_3;
    b.label(LEFT, line, "Work days");
    for ((i, day), label) in (0..).zip(Weekday::ALL).zip(DAY_LABELS) {
        b.add(
            DAY + i,
            Place::at(FIELD + DAY_STEP * i32::from(i), line),
            Kind::Day {
                label: label.into(),
                on: form.work_days.contains(day),
            },
        );
    }
    let line = top + ROW_FIELD + SPACE_3;
    b.label(LEFT, line, "Work hours");
    let time = |t: TimeOfDay| Kind::Time {
        minutes: t.minutes() as i32,
        step: 15,
    };
    b.add(FROM, Place::at(FIELD, line), time(form.start));
    let to = FIELD + STEPPER + SPACE_2;
    b.label(to, line, "to");
    b.add(TO, Place::at(to + TO_W + SPACE_2, line), time(form.end));

    // Nudges: the profile, then one row per nudge.
    let rows = [ROW_NOTED, ROW_NUDGE, ROW_NUDGE, ROW_NUDGE];
    let top = b.section(&mut y, "Nudges", &rows);
    b.noted(top, "Profile", "Fills the numbers below.");
    let entries = Profile::ALL
        .map(|p| Entry::new(profile_name(p), Some(&menu_note(p))))
        .to_vec();
    b.add(
        PROFILE,
        Place::new(
            RIGHT - DROP_W,
            top + (ROW_NOTED - SIZE_FIELD) / 2,
            DROP_W,
            0,
        ),
        Kind::DropDown {
            entries,
            current: shown_profile(form),
            other: "Custom".into(),
        },
    );
    let top = top + ROW_NOTED;
    let stand_ranges = [&config::STAND_EVERY, &config::STAND_LENGTH];
    b.away(
        top,
        Glyph::Stand,
        "Stand up and move",
        &STAND,
        &form.stand,
        stand_ranges,
    );
    let top = top + ROW_NUDGE;
    let pause_ranges = [&config::PAUSE_EVERY, &config::PAUSE_LENGTH];
    b.away(
        top,
        Glyph::Pause,
        "Pause",
        &PAUSE,
        &form.pause,
        pause_ranges,
    );
    let top = top + ROW_NUDGE;
    let line = b.nudge(
        top,
        Glyph::Water,
        "Drink some water",
        WATER_ON,
        form.water.on,
    );
    b.label(NAME, line, "At most every");
    let every = NAME + AT_MOST_W;
    b.add(
        WATER_EVERY,
        Place::at(every, line),
        stepper(&form.water.every, &config::WATER_EVERY, CADENCE_STEP),
    );
    // One line reserved, which the note fills at every scale with Segoe UI Variable; a second
    // line, with another font, grows the card.
    let note = every + STEPPER + SPACE_2;
    b.add(
        FIXED,
        Place::new(note, line + (SIZE_FIELD - NOTE) / 2, RIGHT - note, NOTE),
        Kind::text(
            "Always gentle; rides along with the next nudge.",
            Style::Caption,
            Ink::Muted,
        ),
    );

    // General: Start with Windows.
    let top = b.section(&mut y, "General", &[ROW_NOTED]);
    b.noted(
        top,
        "Start with Windows",
        "breakpoint starts in the tray when you sign in.",
    );
    b.add(
        START_WITH_WINDOWS,
        Place::at(RIGHT - SWITCH_W, top + (ROW_NOTED - SWITCH_H) / 2),
        Kind::Switch {
            on: start_with_windows,
        },
    );

    // The footer band: Save, then Cancel, at the right.
    y += PAD;
    b.add(FIXED, Place::new(0, y, WIDTH, BAND), Kind::Band);
    let cancel_x = WIDTH - PAD - BUTTON_W;
    let save_x = cancel_x - SPACE_2 - BUTTON_W;
    b.add(
        SAVE,
        Place::new(save_x, y + SPACE_4, BUTTON_W, 0),
        Kind::Button {
            label: "Save".into(),
            kind: ButtonKind::Primary,
            default: true,
            cancel: false,
            icon: None,
        },
    );
    b.add(
        CANCEL,
        Place::new(cancel_x, y + SPACE_4, BUTTON_W, 0),
        Kind::Button {
            label: "Cancel".into(),
            kind: ButtonKind::Secondary,
            default: false,
            cancel: true,
            icon: None,
        },
    );
    b.0
}

// ---- Opening and closing ----

/// Opens the settings window filled with `form`, or brings it to the front if it is open
/// (SPEC §9.2). `owner` receives [`WM_SETTINGS_SAVED`] or [`WM_SETTINGS_CANCELLED`]. With `at`,
/// the window's top-left goes there, kept on its monitor; otherwise it is centred. The focus
/// starts on the first work day.
pub fn open(owner: HWND, form: &Form, start_with_windows: bool, at: Option<POINT>) {
    if to_front() {
        return;
    }
    OWNER.set(Some(owner));
    if let Some(handle) = create(form, start_with_windows, at) {
        handle.show();
    }
}

/// Makes the window, hidden.
fn create(form: &Form, start_with_windows: bool, at: Option<POINT>) -> Option<Handle> {
    let spec = Spec {
        caption: CAPTION.into(),
        width: WIDTH,
        height: 0,
        items: items(form, start_with_windows),
        at,
        // No owner, so the window has its own taskbar button.
        owner: None,
        frame: Frame::Dialog,
    };
    let handle = window::open(spec, on_event).ok()?;
    OPEN.set(Some(handle));
    Some(handle)
}

/// The window while it is open.
fn current() -> Option<Handle> {
    OPEN.get().filter(|h| h.is_open())
}

/// Brings the settings window to the front; false if it is not open.
pub fn to_front() -> bool {
    let Some(handle) = current() else {
        return false;
    };
    // SAFETY: the window is live.
    unsafe {
        if IsIconic(handle.hwnd()).as_bool() {
            let _ = ShowWindow(handle.hwnd(), SW_RESTORE);
        }
        let _ = SetForegroundWindow(handle.hwnd());
    }
    true
}

/// The Save that [`WM_SETTINGS_SAVED`] announced, once.
pub fn take_saved() -> Option<Saved> {
    SAVED.take()
}

fn on_event(handle: Handle, event: Event) {
    match event {
        Event::Pressed(SAVE) => save(handle),
        Event::Pressed(CANCEL) | Event::Close => cancel(handle),
        Event::Changed(PROFILE, Value::Index(Some(k))) => pick_profile(handle, k),
        Event::Changed(..) => show_profile(handle),
        Event::Pressed(_) => {}
    }
}

/// Posts `msg` to the owner.
fn post(msg: u32) {
    if let Some(owner) = OWNER.get() {
        // SAFETY: `owner` is the app's main window, alive as long as the app.
        let _ = unsafe { PostMessageW(Some(owner), msg, WPARAM(0), LPARAM(0)) };
    }
}

fn close(handle: Handle) {
    OPEN.set(None);
    handle.close();
}

/// Save: refuse with one message box and the focus on the control, or hand the settings over
/// and close.
fn save(handle: Handle) {
    match read(handle).validate() {
        Err(invalid) => {
            // SAFETY: the window is live; the strings live through the call.
            unsafe {
                MessageBoxW(
                    Some(handle.hwnd()),
                    &HSTRING::from(invalid.message),
                    &HSTRING::from(CAPTION),
                    MB_OK | MB_ICONWARNING,
                );
            }
            handle.focus(control_of(invalid.field));
        }
        Ok(settings) => {
            SAVED.set(Some(Saved {
                settings,
                start_with_windows: on(handle, START_WITH_WINDOWS),
            }));
            post(WM_SETTINGS_SAVED);
            close(handle);
        }
    }
}

/// Cancel, Esc or ×: discard the edits, and tell the owner, for whom it may end first run.
fn cancel(handle: Handle) {
    post(WM_SETTINGS_CANCELLED);
    close(handle);
}

/// The control a refused Save focuses.
fn control_of(field: Field) -> Id {
    match field {
        Field::WorkDays => DAY,
        Field::WorkHoursEnd => TO,
        Field::StandEvery => STAND.every,
        Field::StandLength => STAND.length,
        Field::PauseEvery => PAUSE.every,
        Field::PauseLength => PAUSE.length,
        Field::WaterEvery => WATER_EVERY,
    }
}

// ---- The controls and the core's form ----

fn on(handle: Handle, id: Id) -> bool {
    handle.value(id) == Some(Value::On(true))
}

/// A stepper's number as the core's form holds it.
fn number(handle: Handle, id: Id) -> String {
    match handle.value(id) {
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

fn time(handle: Handle, id: Id) -> Option<TimeOfDay> {
    match handle.value(id)? {
        Value::Number(m) => TimeOfDay::new((m / 60) as u8, (m % 60) as u8),
        _ => None,
    }
}

/// The controls as the core's form.
fn read(handle: Handle) -> Form {
    let mut form = Form::new(&Default::default());
    for (i, day) in (0..).zip(Weekday::ALL) {
        form.work_days.set(day, on(handle, DAY + i));
    }
    form.start = time(handle, FROM).unwrap_or(form.start);
    form.end = time(handle, TO).unwrap_or(form.end);
    let row = |row: &Row| AwayRow {
        on: on(handle, row.on),
        every: number(handle, row.every),
        length: number(handle, row.length),
        enforcement: match handle.value(row.level) {
            Some(Value::Index(Some(1))) => Enforcement::Firm,
            _ => Enforcement::Soft,
        },
    };
    form.stand = row(&STAND);
    form.pause = row(&PAUSE);
    form.water.on = on(handle, WATER_ON);
    form.water.every = number(handle, WATER_EVERY);
    form
}

/// Shows in the drop-down the profile the controls match, or Custom.
fn show_profile(handle: Handle) {
    let shown = Value::Index(shown_profile(&read(handle)));
    if handle.value(PROFILE) != Some(shown) {
        handle.set(PROFILE, shown);
    }
}

/// A profile picked in the drop-down fills the cadences, lengths and levels; the switches and
/// the work week stay as they are, and nothing is saved.
fn pick_profile(handle: Handle, k: usize) {
    let Some(&profile) = Profile::ALL.get(k) else {
        return;
    };
    let mut form = read(handle);
    form.apply(profile);
    let number = |text: &str| Value::Number(text.trim().parse().unwrap_or_default());
    for (row, values) in [(&STAND, &form.stand), (&PAUSE, &form.pause)] {
        handle.set(row.every, number(&values.every));
        handle.set(row.length, number(&values.length));
        handle.set(
            row.level,
            Value::Index(Some(level_index(values.enforcement))),
        );
    }
    handle.set(WATER_EVERY, number(&form.water.every));
    show_profile(handle);
}

#[cfg(test)]
mod tests;
