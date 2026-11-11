//! The form's state machine, in plain Rust: layout at a DPI, hit-testing, focus order, keys,
//! stepper math, the drop-down menu and the hover, press and knob timing.
//!
//! The window feeds it keys, pointer moves and frame ticks, and hands the [`Event`]s it returns
//! to the caller. Nothing here touches a window, so unit tests drive all of it.

use std::time::Duration;

use windows::Win32::Foundation::RECT;
use windows::Win32::UI::WindowsAndMessaging::USER_DEFAULT_SCREEN_DPI as BASE_DPI;

use super::{ButtonKind, Entry, Event, Id, Item, Kind, Value};
use crate::platform::paint::{Box2, HOVER, PRESSED, REST, step_fill};
use crate::platform::theme::{
    CHECKBOX, CHEVRON, CHEVRON_RIGHT, CHOICE_PAD_X, CHOICE_PAD_Y, HAIRLINE, KNOB_SLIDE, MENU_CHECK,
    MENU_ITEM_PAD_X, MENU_ITEM_PAD_Y, MENU_PAD, PAGE_STEPS, PILL_H, PILL_W, PRIMARY_MIN_WIDTH,
    RADIO, SCROLLBAR_HOT_W, SCROLLBAR_INSET, SCROLLBAR_MIN, SCROLLBAR_W, SEGMENT_GAP, SEGMENT_H,
    SEGMENT_MIN_W, SEGMENT_PAD, SIZE_BUTTON, SIZE_FIELD, SIZE_GLYPH, SPACE_2, SPACE_3, SPACE_4,
    STEPPER_VALUE, STEPPER_ZONE, SWITCH_H, SWITCH_W, Style, px,
};

/// Minutes in a day: time steppers wrap round it.
pub const DAY: i32 = 24 * 60;
/// The longest number a stepper takes typed.
const NUMBER_CHARS: usize = 4;
/// The longest time a time stepper takes typed: `HH:mm`.
const TIME_CHARS: usize = 5;

/// Text widths in pixels, so the layout can size text, buttons and menus.
pub trait Measure {
    fn width(&self, style: Style, text: &str) -> i32;
}

/// The parts of a control that answer the pointer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    /// The whole control.
    Body,
    /// A stepper's − zone, + zone and value between them.
    Minus,
    Plus,
    Field,
    /// One segment of a segmented toggle.
    Segment(usize),
}

impl Part {
    /// The index of this part's hover level in a [`Look`].
    pub const fn slot(self) -> usize {
        match self {
            Part::Body | Part::Minus => 0,
            Part::Plus => 1,
            Part::Field => 2,
            Part::Segment(i) => i,
        }
    }
}

/// An item laid out at the form's DPI, in client pixels.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Laid {
    pub rect: RECT,
    /// A text item's lines after wrapping.
    pub lines: Vec<String>,
    /// The parts that answer the pointer.
    pub parts: Vec<(Part, RECT)>,
    /// A card's row hairlines, as client y.
    pub rows: Vec<i32>,
}

/// A key the form handles. The window turns Win32 key messages into these.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Tab,
    /// Shift+Tab.
    BackTab,
    Enter,
    Esc,
    Space,
    Up,
    Down,
    Left,
    Right,
    PageUp,
    PageDown,
    Home,
    End,
    AltDown,
    AltUp,
    Back,
    /// A typed character, such as a digit for a stepper.
    Char(char),
}

/// The open drop-down menu: which item owns it and which entry is highlighted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Menu {
    pub item: usize,
    pub hot: Option<usize>,
}

/// How to paint one item now: its hover levels per part (0 rest, 1 hover, 2 press), the switch
/// knob's place (0 off, 1 on), the focus ring, whether its menu is open, and typed text.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Look {
    pub levels: Vec<f32>,
    pub slide: f32,
    pub ring: bool,
    pub open: bool,
    pub edit: Option<String>,
}

type Target = (usize, Part);

/// Which part the pointer is over, and which part a press started on. A press acts only when
/// it is released on the part it started on, as in a native button.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct Pointer {
    hot: Option<Target>,
    pressed: Option<Target>,
}

impl Pointer {
    fn moved(&mut self, hit: Option<Target>, down: bool) -> bool {
        let before = *self;
        if !down {
            self.pressed = None;
        }
        self.hot = hit;
        *self != before
    }

    fn left(&mut self, down: bool) {
        self.hot = None;
        if !down {
            self.pressed = None;
        }
    }

    fn down(&mut self, hit: Option<Target>) {
        self.hot = hit;
        self.pressed = hit;
    }

    fn up(&mut self, hit: Option<Target>) -> Option<Target> {
        self.hot = hit;
        self.pressed.take().filter(|&p| Some(p) == hit)
    }

    fn target(&self, t: Target) -> f32 {
        match (self.hot == Some(t), self.pressed) {
            (true, Some(p)) if p == t => PRESSED,
            (true, None) => HOVER,
            _ => REST,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
struct Anim {
    levels: Vec<f32>,
    slide: f32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Edit {
    item: usize,
    text: String,
    /// The value when typing began, which text that is no number gives back on commit.
    before: i32,
}

/// A form: its items and everything the user has done to them.
#[derive(Debug)]
pub struct Form {
    items: Vec<Item>,
    laid: Vec<Laid>,
    dpi: u32,
    focus: Option<usize>,
    /// Whether the window has the keyboard focus.
    active: bool,
    /// Whether the focus ring shows: only after keyboard use, like `UISF_HIDEFOCUS`.
    cues: bool,
    pointer: Pointer,
    anim: Vec<Anim>,
    edit: Option<Edit>,
    menu: Option<Menu>,
    /// Where wrapped text needs more room than its place gives at this DPI: the y at 96 DPI
    /// under which items move down, and by how many pixels.
    growth: Vec<(i32, i32)>,
}

impl Form {
    /// A form of `items`, focus on the first Tab stop. Lay it out before use.
    pub fn new(items: Vec<Item>) -> Form {
        let anim = items
            .iter()
            .map(|item| Anim {
                levels: vec![REST; slots(&item.kind)],
                slide: on_slide(&item.kind),
            })
            .collect();
        let mut form = Form {
            laid: vec![Laid::default(); items.len()],
            items,
            dpi: BASE_DPI,
            focus: None,
            active: false,
            cues: false,
            pointer: Pointer::default(),
            anim,
            edit: None,
            menu: None,
            growth: Vec::new(),
        };
        form.focus = form.tab_stops().first().copied();
        form
    }

    pub fn items(&self) -> &[Item] {
        &self.items
    }

    pub fn laid(&self) -> &[Laid] {
        &self.laid
    }

    pub fn dpi(&self) -> u32 {
        self.dpi
    }

    pub fn index(&self, id: Id) -> Option<usize> {
        self.items.iter().position(|item| item.id == id)
    }

    /// The focused item's id.
    pub fn focused(&self) -> Option<Id> {
        self.focus.map(|i| self.items[i].id)
    }

    pub fn menu(&self) -> Option<Menu> {
        self.menu
    }

    // ---- Layout ----

    /// Lays every item out at `dpi`: places scaled with [`px`], own sizes from the controls
    /// table, text measured with `m`.
    ///
    /// A wrapped text whose place gives a height reserves that room. When it needs more lines
    /// at this DPI, every item placed at or under its bottom moves down by the extra, and a
    /// card or band round its bottom grows by it, so the form keeps its reading order and
    /// [`Form::height`] grows with it.
    pub fn lay_out(&mut self, dpi: u32, m: &dyn Measure) {
        self.dpi = dpi;
        self.laid = self
            .items
            .iter()
            .map(|item| lay_out(item, dpi, m))
            .collect();
        self.growth = self
            .items
            .iter()
            .zip(&self.laid)
            .filter_map(|(item, laid)| match item.kind {
                Kind::Text { .. } | Kind::Choice { .. } if item.at.w > 0 && item.at.h > 0 => {
                    let extra = laid.rect.bottom - laid.rect.top - px(item.at.h, dpi);
                    (extra > 0).then_some((item.at.y + item.at.h, extra))
                }
                _ => None,
            })
            .collect();
        let growth = &self.growth;
        for (item, laid) in self.items.iter().zip(&mut self.laid) {
            let (top, bottom) = (item.at.y, item.at.y + item.at.h);
            let down = shift(growth, top);
            let grow = shift(growth, bottom) - down;
            let grows = matches!(item.kind, Kind::Card { .. } | Kind::Band);
            let move_rect = |r: &mut RECT| {
                r.top += down;
                r.bottom += down;
            };
            move_rect(&mut laid.rect);
            laid.parts.iter_mut().for_each(|(_, r)| move_rect(r));
            if grows {
                laid.rect.bottom += grow;
            }
            if let Kind::Card { rows } = &item.kind {
                laid.rows = rows
                    .iter()
                    .map(|&row| laid.rect.top + px(row, dpi) + shift(growth, top + row) - down)
                    .collect();
            }
        }
    }

    /// The client height for a form `height` tall at 96 DPI, grown for wrapped text. With a
    /// `height` of 0, the bottom of the lowest item.
    pub fn height(&self, height: i32) -> i32 {
        if height > 0 {
            px(height, self.dpi) + shift(&self.growth, height)
        } else {
            self.laid.iter().map(|l| l.rect.bottom).max().unwrap_or(0)
        }
    }

    /// The top of the footer band in pixels: the last [`Kind::Band`]. It and the items under
    /// its top stay in view while the rest of a form taller than its window scrolls. `None`
    /// when the form has no band.
    pub fn footer(&self) -> Option<i32> {
        self.items
            .iter()
            .zip(&self.laid)
            .rev()
            .find(|(item, _)| matches!(item.kind, Kind::Band))
            .map(|(_, laid)| laid.rect.top)
    }

    /// The focused item's laid-out rectangle.
    pub fn focus_rect(&self) -> Option<RECT> {
        self.focus.map(|i| self.laid[i].rect)
    }

    /// The item and part under (`x`, `y`) in form pixels (see [`Scroll::in_form`]). Later
    /// items win.
    pub fn hit(&self, x: i32, y: i32) -> Option<(usize, Part)> {
        self.laid.iter().enumerate().rev().find_map(|(i, laid)| {
            if !self.items[i].kind.focusable() {
                return None;
            }
            laid.parts
                .iter()
                .find(|(_, r)| inside(r, x, y))
                .map(|&(part, _)| (i, part))
        })
    }

    // ---- Values ----

    pub fn value(&self, id: Id) -> Option<Value> {
        Some(match &self.items[self.index(id)?].kind {
            Kind::Switch { on } | Kind::Checkbox { on, .. } | Kind::Day { on, .. } => {
                Value::On(*on)
            }
            Kind::Choice { selected, .. } => Value::On(*selected),
            Kind::Stepper { value, .. } => Value::Number(*value),
            Kind::Time { minutes, .. } => Value::Number(*minutes),
            Kind::Segmented { selected, .. } => Value::Index(Some(*selected)),
            Kind::DropDown { current, .. } => Value::Index(*current),
            _ => return None,
        })
    }

    /// Sets a control's value without an event. Returns false when the item takes no such value.
    pub fn set(&mut self, id: Id, value: Value) -> bool {
        let Some(i) = self.index(id) else {
            return false;
        };
        if self.edit.as_ref().is_some_and(|e| e.item == i) {
            self.edit = None;
        }
        match (&mut self.items[i].kind, value) {
            (
                Kind::Switch { on } | Kind::Checkbox { on, .. } | Kind::Day { on, .. },
                Value::On(v),
            ) => *on = v,
            (Kind::Stepper { value, .. }, Value::Number(v)) => *value = v,
            (Kind::Time { minutes, .. }, Value::Number(v)) => *minutes = v.rem_euclid(DAY),
            (Kind::Segmented { options, selected }, Value::Index(Some(v))) if v < options.len() => {
                *selected = v
            }
            (
                Kind::DropDown {
                    entries, current, ..
                },
                Value::Index(v),
            ) if v.is_none_or(|v| v < entries.len()) => *current = v,
            (Kind::Choice { .. }, Value::On(true)) => self.select_choice(i),
            (Kind::Choice { selected, .. }, Value::On(false)) => *selected = false,
            _ => return false,
        }
        true
    }

    /// Changes a text item's text, a button's label or a day pill's label. Lay out again after.
    pub fn set_text(&mut self, id: Id, text: &str) -> bool {
        let Some(i) = self.index(id) else {
            return false;
        };
        match &mut self.items[i].kind {
            Kind::Text { text: t, .. }
            | Kind::Button { label: t, .. }
            | Kind::Day { label: t, .. }
            | Kind::Checkbox { label: t, .. } => *t = text.to_owned(),
            Kind::DropDown { other, .. } => *other = text.to_owned(),
            _ => return false,
        }
        true
    }

    // ---- Focus ----

    fn is_tab_stop(&self, i: usize) -> bool {
        match &self.items[i].kind {
            Kind::Choice {
                group, selected, ..
            } => *selected || (!self.group_has_selection(*group) && self.group(*group)[0] == i),
            kind => kind.focusable(),
        }
    }

    /// The items Tab stops at, in list order. A selectable card group is one stop: its selected
    /// card, or its first when none is selected.
    pub fn tab_stops(&self) -> Vec<usize> {
        (0..self.items.len())
            .filter(|&i| self.is_tab_stop(i))
            .collect()
    }

    fn group(&self, group: u16) -> Vec<usize> {
        (0..self.items.len())
            .filter(|&i| matches!(self.items[i].kind, Kind::Choice { group: g, .. } if g == group))
            .collect()
    }

    fn group_has_selection(&self, group: u16) -> bool {
        self.items.iter().any(
            |item| matches!(item.kind, Kind::Choice { group: g, selected: true, .. } if g == group),
        )
    }

    /// Moves the focus to item `i`, committing typed text and closing the menu on the way.
    fn focus_on(&mut self, i: usize) -> Vec<Event> {
        if self.focus == Some(i) {
            return Vec::new();
        }
        let out = self.commit();
        self.menu = None;
        self.focus = Some(i);
        out
    }

    /// Focuses the item `id` and shows the focus ring, as after a refused Save: the user must
    /// see which control to fix, even when Save was clicked.
    pub fn focus(&mut self, id: Id) -> Vec<Event> {
        self.cues = true;
        match self.index(id) {
            Some(i) if self.items[i].kind.focusable() => self.focus_on(i),
            _ => Vec::new(),
        }
    }

    fn move_focus(&mut self, forward: bool) -> Vec<Event> {
        let stops = self.tab_stops();
        // From inside a card group, Tab leaves the whole group, whichever card has the focus.
        let cur = self.focus.map(|c| match self.items[c].kind {
            Kind::Choice { group, .. } => {
                let members = self.group(group);
                if forward {
                    members.last().copied().unwrap_or(c)
                } else {
                    members.first().copied().unwrap_or(c)
                }
            }
            _ => c,
        });
        let next = if forward {
            stops
                .iter()
                .find(|&&s| cur.is_none_or(|c| s > c))
                .or(stops.first())
        } else {
            stops
                .iter()
                .rev()
                .find(|&&s| cur.is_none_or(|c| s < c))
                .or(stops.last())
        };
        match next {
            Some(&i) => self.focus_on(i),
            None => Vec::new(),
        }
    }

    /// The window got the keyboard focus.
    pub fn focus_in(&mut self) {
        self.active = true;
    }

    /// The window lost the keyboard focus: typed text is committed and the menu closes.
    pub fn focus_out(&mut self) -> Vec<Event> {
        self.active = false;
        self.menu = None;
        self.commit()
    }

    /// Shows the focus ring from now on, as after keyboard use.
    pub fn show_cues(&mut self) {
        self.cues = true;
    }

    // ---- Keys ----

    /// Handles `key`; returns what the user did.
    pub fn key(&mut self, key: Key) -> Vec<Event> {
        self.cues = true;
        if let Some(menu) = self.menu {
            return self.menu_key(menu, key);
        }
        match key {
            Key::Tab => return self.move_focus(true),
            Key::BackTab => return self.move_focus(false),
            Key::Enter => {
                // As in a Windows dialog: Enter presses the focused button, and otherwise the
                // default button, also on a closed drop-down (Space and Alt+Down open it).
                if let Some(i) = self.focus
                    && let Kind::Button { .. } = self.items[i].kind
                {
                    return vec![Event::Pressed(self.items[i].id)];
                }
                let mut out = self.commit();
                out.extend(self.button(|default, _| default).map(Event::Pressed));
                return out;
            }
            Key::Esc => {
                // One Esc cancels, also while typing: the typed text goes with the other edits.
                let mut out = self.drop_edit();
                out.push(self.escape());
                return out;
            }
            _ => {}
        }
        let Some(i) = self.focus else {
            return Vec::new();
        };
        match key {
            Key::Space => self.activate(i),
            Key::Up | Key::Left | Key::Down | Key::Right => {
                let forward = matches!(key, Key::Down | Key::Right);
                match &self.items[i].kind {
                    Kind::Segmented { .. } => self.segment_by(i, forward),
                    Kind::Choice { .. } => self.choice_by(i, forward),
                    Kind::Stepper { .. } | Kind::Time { .. } if matches!(key, Key::Up) => {
                        self.step(i, Step::By(1))
                    }
                    Kind::Stepper { .. } | Kind::Time { .. } if matches!(key, Key::Down) => {
                        self.step(i, Step::By(-1))
                    }
                    _ => Vec::new(),
                }
            }
            Key::PageUp => self.step(i, Step::By(PAGE_STEPS)),
            Key::PageDown => self.step(i, Step::By(-PAGE_STEPS)),
            Key::Home => self.step(i, Step::Min),
            Key::End => self.step(i, Step::Max),
            Key::AltDown => {
                if matches!(self.items[i].kind, Kind::DropDown { .. }) {
                    self.open_menu(i);
                }
                Vec::new()
            }
            Key::Back => self.erase(i),
            Key::Char(c) => self.type_char(i, c),
            _ => Vec::new(),
        }
    }

    /// Esc or the caption ×: presses the cancel button, or asks to close.
    pub fn escape(&self) -> Event {
        self.button(|_, cancel| cancel)
            .map_or(Event::Close, Event::Pressed)
    }

    /// The first button whose (default, cancel) flags pass `which`.
    fn button(&self, which: impl Fn(bool, bool) -> bool) -> Option<Id> {
        self.items.iter().find_map(|item| match item.kind {
            Kind::Button {
                default, cancel, ..
            } if which(default, cancel) => Some(item.id),
            _ => None,
        })
    }

    /// Space on item `i`, or a click released on it.
    fn activate(&mut self, i: usize) -> Vec<Event> {
        let id = self.items[i].id;
        match &mut self.items[i].kind {
            Kind::Button { .. } => vec![Event::Pressed(id)],
            Kind::Switch { on } | Kind::Checkbox { on, .. } | Kind::Day { on, .. } => {
                *on = !*on;
                vec![Event::Changed(id, Value::On(*on))]
            }
            Kind::DropDown { .. } => {
                self.open_menu(i);
                Vec::new()
            }
            Kind::Choice {
                selected: false, ..
            } => {
                self.select_choice(i);
                let mut out = self.focus_on(i);
                out.push(Event::Changed(id, Value::On(true)));
                out
            }
            _ => Vec::new(),
        }
    }

    fn segment_by(&mut self, i: usize, forward: bool) -> Vec<Event> {
        let id = self.items[i].id;
        let Kind::Segmented { options, selected } = &mut self.items[i].kind else {
            return Vec::new();
        };
        let n = options.len();
        if n == 0 {
            return Vec::new();
        }
        *selected = if forward {
            (*selected + 1) % n
        } else {
            (*selected + n - 1) % n
        };
        vec![Event::Changed(id, Value::Index(Some(*selected)))]
    }

    fn select_segment(&mut self, i: usize, k: usize) -> Vec<Event> {
        let id = self.items[i].id;
        match &mut self.items[i].kind {
            Kind::Segmented { options, selected } if k < options.len() && *selected != k => {
                *selected = k;
                vec![Event::Changed(id, Value::Index(Some(k)))]
            }
            _ => Vec::new(),
        }
    }

    /// Makes card `i` the selected one of its group.
    fn select_choice(&mut self, i: usize) {
        let Kind::Choice { group, .. } = self.items[i].kind else {
            return;
        };
        for j in self.group(group) {
            if let Kind::Choice { selected, .. } = &mut self.items[j].kind {
                *selected = i == j;
            }
        }
    }

    fn choice_by(&mut self, i: usize, forward: bool) -> Vec<Event> {
        let Kind::Choice { group, .. } = self.items[i].kind else {
            return Vec::new();
        };
        let members = self.group(group);
        let at = members.iter().position(|&j| j == i).unwrap_or(0);
        let n = members.len();
        let next = members[if forward {
            (at + 1) % n
        } else {
            (at + n - 1) % n
        }];
        self.select_choice(next);
        self.focus = Some(next);
        vec![Event::Changed(self.items[next].id, Value::On(true))]
    }

    // ---- Steppers ----

    fn step(&mut self, i: usize, how: Step) -> Vec<Event> {
        let mut out = self.commit();
        let id = self.items[i].id;
        let changed = match &mut self.items[i].kind {
            Kind::Stepper {
                value,
                min,
                max,
                step,
                ..
            } => {
                let next = match how {
                    Step::By(n) => stepped(*value, *step, n, *min, *max),
                    Step::Min => *min,
                    Step::Max => *max,
                };
                (next != *value).then(|| {
                    *value = next;
                    next
                })
            }
            Kind::Time { minutes, step } => {
                let next = match how {
                    Step::By(n) => stepped_time(*minutes, *step, n),
                    Step::Min => 0,
                    Step::Max => DAY - 1,
                };
                (next != *minutes).then(|| {
                    *minutes = next;
                    next
                })
            }
            _ => None,
        };
        out.extend(changed.map(|v| Event::Changed(id, Value::Number(v))));
        out
    }

    /// The number or time item `i` shows when nothing is typed.
    fn shown(&self, i: usize) -> Option<i32> {
        match self.items[i].kind {
            Kind::Stepper { value, .. } => Some(value),
            Kind::Time { minutes, .. } => Some(minutes),
            _ => None,
        }
    }

    fn type_char(&mut self, i: usize, c: char) -> Vec<Event> {
        let (limit, allowed) = match self.items[i].kind {
            Kind::Stepper { .. } => (NUMBER_CHARS, c.is_ascii_digit()),
            Kind::Time { .. } => (TIME_CHARS, c.is_ascii_digit() || c == ':'),
            _ => return Vec::new(),
        };
        let Some(before) = self.shown(i).filter(|_| allowed) else {
            return Vec::new();
        };
        let edit = self.edit.get_or_insert_with(|| Edit {
            item: i,
            text: String::new(),
            before,
        });
        if edit.text.len() < limit {
            edit.text.push(c);
        }
        self.take_typed(i)
    }

    fn erase(&mut self, i: usize) -> Vec<Event> {
        let Some(before) = self.shown(i) else {
            return Vec::new();
        };
        let shown = match self.items[i].kind {
            Kind::Time { minutes, .. } => time_text(minutes),
            _ => before.to_string(),
        };
        let edit = self.edit.get_or_insert(Edit {
            item: i,
            text: shown,
            before,
        });
        edit.text.pop();
        self.take_typed(i)
    }

    /// A number stepper takes each typed number at once and reports it, so the caller can
    /// follow the typing, as the settings window's profile does (SPEC §9.2). A time stepper
    /// waits for the commit, since a part of a time is a different time.
    fn take_typed(&mut self, i: usize) -> Vec<Event> {
        let (Some(edit), id) = (&self.edit, self.items[i].id) else {
            return Vec::new();
        };
        match &mut self.items[i].kind {
            Kind::Stepper { value, .. } => parse_number(&edit.text)
                .filter(|v| v != value)
                .map(|v| {
                    *value = v;
                    vec![Event::Changed(id, Value::Number(v))]
                })
                .unwrap_or_default(),
            _ => Vec::new(),
        }
    }

    /// Ends typing in a stepper: a number keeps what was typed, even out of range; a time keeps
    /// it if it is a real time. Anything else gives back the value from before the typing.
    fn commit(&mut self) -> Vec<Event> {
        let Some(edit) = self.edit.take() else {
            return Vec::new();
        };
        let parsed = match self.items[edit.item].kind {
            Kind::Stepper { .. } => parse_number(&edit.text),
            Kind::Time { .. } => parse_time(&edit.text),
            _ => None,
        };
        self.set_typed(edit.item, parsed.unwrap_or(edit.before))
    }

    /// Drops the typing and gives back the value from before it.
    fn drop_edit(&mut self) -> Vec<Event> {
        match self.edit.take() {
            Some(edit) => self.set_typed(edit.item, edit.before),
            None => Vec::new(),
        }
    }

    /// Sets a stepper's value to `v`, reporting it when it changes.
    fn set_typed(&mut self, i: usize, v: i32) -> Vec<Event> {
        let id = self.items[i].id;
        let changed = match &mut self.items[i].kind {
            Kind::Stepper { value, .. } if *value != v => {
                *value = v;
                true
            }
            Kind::Time { minutes, .. } if *minutes != v => {
                *minutes = v;
                true
            }
            _ => false,
        };
        if changed {
            vec![Event::Changed(id, Value::Number(v))]
        } else {
            Vec::new()
        }
    }

    // ---- Drop-down menu ----

    fn open_menu(&mut self, i: usize) {
        if let Kind::DropDown {
            entries, current, ..
        } = &self.items[i].kind
            && !entries.is_empty()
        {
            self.menu = Some(Menu {
                item: i,
                hot: Some(current.unwrap_or(0)),
            });
        }
    }

    pub fn close_menu(&mut self) {
        self.menu = None;
    }

    /// The open menu's entries.
    pub fn menu_entries(&self) -> Option<(&[Entry], Option<usize>)> {
        let menu = self.menu?;
        match &self.items[menu.item].kind {
            Kind::DropDown {
                entries, current, ..
            } => Some((entries, *current)),
            _ => None,
        }
    }

    fn menu_key(&mut self, menu: Menu, key: Key) -> Vec<Event> {
        let n = self.menu_entries().map_or(0, |(e, _)| e.len());
        let hot = menu.hot;
        let set_hot = |form: &mut Form, h: usize| {
            form.menu = Some(Menu {
                hot: Some(h.min(n.saturating_sub(1))),
                ..menu
            })
        };
        match key {
            Key::Up => set_hot(self, hot.map_or(0, |h| h.saturating_sub(1))),
            Key::Down => set_hot(self, hot.map_or(0, |h| h + 1)),
            Key::Home | Key::PageUp => set_hot(self, 0),
            Key::End | Key::PageDown => set_hot(self, n.saturating_sub(1)),
            Key::Enter => {
                return match hot {
                    Some(h) => self.menu_pick(h),
                    None => {
                        self.menu = None;
                        Vec::new()
                    }
                };
            }
            Key::Esc | Key::AltUp | Key::AltDown => self.menu = None,
            Key::Tab | Key::BackTab => {
                self.menu = None;
                return self.key(key);
            }
            _ => {}
        }
        Vec::new()
    }

    /// The pointer is over menu entry `hot`, or over none.
    pub fn menu_hover(&mut self, hot: Option<usize>) -> bool {
        match &mut self.menu {
            Some(menu) if menu.hot != hot && hot.is_some() => {
                menu.hot = hot;
                true
            }
            _ => false,
        }
    }

    /// Picks menu entry `k` and closes the menu.
    pub fn menu_pick(&mut self, k: usize) -> Vec<Event> {
        let Some(menu) = self.menu.take() else {
            return Vec::new();
        };
        let id = self.items[menu.item].id;
        match &mut self.items[menu.item].kind {
            Kind::DropDown {
                entries, current, ..
            } if k < entries.len() && *current != Some(k) => {
                *current = Some(k);
                vec![Event::Changed(id, Value::Index(Some(k)))]
            }
            _ => Vec::new(),
        }
    }

    // ---- Pointer ----

    /// The pointer moved to (`x`, `y`) with the left button `down` or not. True if the look
    /// changed.
    pub fn pointer_move(&mut self, x: i32, y: i32, down: bool) -> bool {
        let hit = self.hit(x, y);
        self.pointer.moved(hit, down)
    }

    pub fn pointer_leave(&mut self, down: bool) {
        self.pointer.left(down);
    }

    /// The left button went down at (`x`, `y`). Steppers step and drop-downs open here; the rest
    /// act on release. With the menu open, the press only closes it.
    pub fn pointer_down(&mut self, x: i32, y: i32) -> Vec<Event> {
        if self.menu.take().is_some() {
            self.pointer.down(None);
            return Vec::new();
        }
        let hit = self.hit(x, y);
        self.pointer.down(hit);
        let Some((i, part)) = hit else {
            return Vec::new();
        };
        // A card that is not selected takes the focus only when the click selects it, on
        // release; a press dragged off it leaves the focus where it was.
        let mut out = if matches!(
            self.items[i].kind,
            Kind::Choice {
                selected: false,
                ..
            }
        ) {
            self.commit()
        } else {
            self.focus_on(i)
        };
        match (&self.items[i].kind, part) {
            (Kind::Stepper { .. } | Kind::Time { .. }, Part::Minus) => {
                out.extend(self.step(i, Step::By(-1)))
            }
            (Kind::Stepper { .. } | Kind::Time { .. }, Part::Plus) => {
                out.extend(self.step(i, Step::By(1)))
            }
            (Kind::DropDown { .. }, _) => self.open_menu(i),
            _ => {}
        }
        out
    }

    /// The left button came up at (`x`, `y`): acts when it is the part the press started on.
    pub fn pointer_up(&mut self, x: i32, y: i32) -> Vec<Event> {
        let hit = self.hit(x, y);
        let Some((i, part)) = self.pointer.up(hit) else {
            return Vec::new();
        };
        match (&self.items[i].kind, part) {
            (Kind::Segmented { .. }, Part::Segment(k)) => self.select_segment(i, k),
            (
                Kind::Button { .. }
                | Kind::Switch { .. }
                | Kind::Checkbox { .. }
                | Kind::Day { .. }
                | Kind::Choice { .. },
                _,
            ) => self.activate(i),
            _ => Vec::new(),
        }
    }

    /// The press ended without a release here, such as when capture was lost.
    pub fn pointer_cancel(&mut self) {
        self.pointer.pressed = None;
    }

    // ---- Motion ----

    fn target(&self, i: usize, slot: usize) -> f32 {
        let (hot, pressed) = (self.pointer.hot, self.pointer.pressed);
        let part = [hot, pressed]
            .into_iter()
            .flatten()
            .find(|&(j, p)| j == i && p.slot() == slot);
        part.map_or(REST, |p| self.pointer.target(p))
    }

    /// Moves every hover level, press and knob `dt` toward where it is heading; at once when
    /// `animate` is false. True while anything still moves.
    pub fn tick(&mut self, dt: Duration, animate: bool) -> bool {
        for i in 0..self.items.len() {
            for slot in 0..self.anim[i].levels.len() {
                let target = self.target(i, slot);
                let level = &mut self.anim[i].levels[slot];
                *level = step_fill(*level, target, dt, animate);
            }
            let to = on_slide(&self.items[i].kind);
            let slide = &mut self.anim[i].slide;
            *slide = if animate {
                let step = dt.as_secs_f32() / KNOB_SLIDE.as_secs_f32();
                if to > *slide {
                    (*slide + step).min(to)
                } else {
                    (*slide - step).max(to)
                }
            } else {
                to
            };
        }
        self.moving()
    }

    pub fn moving(&self) -> bool {
        (0..self.items.len()).any(|i| {
            let a = &self.anim[i];
            a.slide != on_slide(&self.items[i].kind)
                || a.levels
                    .iter()
                    .enumerate()
                    .any(|(slot, &level)| level != self.target(i, slot))
        })
    }

    /// How to paint each item now.
    pub fn looks(&self) -> Vec<Look> {
        (0..self.items.len())
            .map(|i| Look {
                levels: self.anim[i].levels.clone(),
                slide: self.anim[i].slide,
                ring: self.cues && self.active && self.focus == Some(i),
                open: self.menu.is_some_and(|m| m.item == i),
                edit: self
                    .edit
                    .as_ref()
                    .filter(|e| e.item == i)
                    .map(|e| e.text.clone()),
            })
            .collect()
    }
}

#[derive(Clone, Copy, Debug)]
enum Step {
    By(i32),
    Min,
    Max,
}

/// How many hover levels a control keeps.
fn slots(kind: &Kind) -> usize {
    match kind {
        Kind::Stepper { .. } | Kind::Time { .. } => 3,
        Kind::Segmented { options, .. } => options.len().max(1),
        _ => 1,
    }
}

/// Where a switch knob rests: 1 on, 0 off. Other controls keep 0.
fn on_slide(kind: &Kind) -> f32 {
    match kind {
        Kind::Switch { on: true } => 1.0,
        _ => 0.0,
    }
}

pub fn inside(r: &RECT, x: i32, y: i32) -> bool {
    x >= r.left && x < r.right && y >= r.top && y < r.bottom
}

pub fn rect(x: i32, y: i32, w: i32, h: i32) -> RECT {
    RECT {
        left: x,
        top: y,
        right: x + w,
        bottom: y + h,
    }
}

// ---- Stepper math ----

/// `value` moved `count` steps of `step`, snapping to the step's multiples (from 07 by 15: up to
/// 15, down to 00), then held in `min..=max`. A value typed out of range comes back into it.
pub fn stepped(value: i32, step: i32, count: i32, min: i32, max: i32) -> i32 {
    let mut v = value;
    for _ in 0..count.unsigned_abs() {
        v = snap(v, step, count > 0).clamp(min, max);
    }
    v
}

/// As [`stepped`] for minutes after midnight, wrapping round midnight.
pub fn stepped_time(minutes: i32, step: i32, count: i32) -> i32 {
    let mut v = minutes;
    for _ in 0..count.unsigned_abs() {
        v = snap(v, step, count > 0).rem_euclid(DAY);
    }
    v
}

/// The next multiple of `step` above `value`, or the one below.
fn snap(value: i32, step: i32, up: bool) -> i32 {
    let step = step.max(1);
    if up {
        value.div_euclid(step) * step + step
    } else {
        (value - 1).div_euclid(step) * step
    }
}

/// A typed number, or `None` when nothing was typed.
pub fn parse_number(text: &str) -> Option<i32> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

/// A typed time as minutes after midnight: `8`, `08`, `830`, `0830`, `8:30` and `08:30` all
/// work. `None` for anything that is not a time of day.
pub fn parse_time(text: &str) -> Option<i32> {
    let num = |s: &str| {
        (!s.is_empty() && s.len() <= 2)
            .then(|| parse_number(s))
            .flatten()
    };
    let (h, m) = match text.split_once(':') {
        Some((h, m)) => (num(h)?, num(m)?),
        None => match text.len() {
            1 | 2 => (num(text)?, 0),
            3 => (num(&text[..1])?, num(&text[1..])?),
            4 => (num(&text[..2])?, num(&text[2..])?),
            _ => return None,
        },
    };
    (h < 24 && m < 60).then_some(h * 60 + m)
}

/// Minutes after midnight as `HH:mm`.
pub fn time_text(minutes: i32) -> String {
    let m = minutes.rem_euclid(DAY);
    format!("{:02}:{:02}", m / 60, m % 60)
}

/// What a stepper shows: the number (or time) and the unit after it.
pub fn stepper_text(kind: &Kind) -> Option<(String, &str)> {
    match kind {
        Kind::Stepper { value, unit, .. } => Some((value.to_string(), unit.as_str())),
        Kind::Time { minutes, .. } => Some((time_text(*minutes), "")),
        _ => None,
    }
}

// ---- Layout ----

/// `text` broken at spaces into lines no wider than `width`, and at every `\n`. A word wider
/// than `width` gets a line of its own.
pub fn wrap(text: &str, width: i32, fits: impl Fn(&str) -> i32) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        let mut line = String::new();
        for word in paragraph.split_whitespace() {
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
    }
    lines
}

fn lay_out(item: &Item, dpi: u32, m: &dyn Measure) -> Laid {
    let s = |v: i32| px(v, dpi);
    let at = item.at;
    let (x, y) = (s(at.x), s(at.y));
    let given_w = (at.w > 0).then(|| s(at.w));
    let given_h = (at.h > 0).then(|| s(at.h));
    let lh = |style: Style| s(style.line_height());
    let sized = |w: i32, h: i32| rect(x, y, given_w.unwrap_or(w), given_h.unwrap_or(h));
    let mut lines = Vec::new();
    let mut parts = Vec::new();
    let r = match &item.kind {
        Kind::Card { .. } | Kind::Band => sized(0, 0),
        Kind::Text { text, style, .. } => {
            lines = match given_w {
                Some(w) => wrap(text, w, |t| m.width(*style, t)),
                None => vec![text.clone()],
            };
            let widest = lines.iter().map(|l| m.width(*style, l)).max().unwrap_or(0);
            let tall = lines.len() as i32 * lh(*style);
            let r = sized(widest, tall);
            // Never shorter than its lines: the room past its place is growth.
            rect(
                r.left,
                r.top,
                r.right - r.left,
                (r.bottom - r.top).max(tall),
            )
        }
        Kind::Glyph(_) => sized(s(SIZE_GLYPH), s(SIZE_GLYPH)),
        Kind::Button { label, kind, .. } => {
            let w = m.width(Style::BodyStrong, label) + 2 * (s(SPACE_3) + s(HAIRLINE));
            let least = if *kind == ButtonKind::Primary {
                s(PRIMARY_MIN_WIDTH)
            } else {
                0
            };
            sized(w.max(least), s(SIZE_BUTTON))
        }
        Kind::Switch { .. } => sized(s(SWITCH_W), s(SWITCH_H)),
        Kind::Checkbox { label, .. } => {
            if label.is_empty() {
                sized(s(CHECKBOX), s(CHECKBOX))
            } else {
                let w = s(CHECKBOX) + s(SPACE_2) + m.width(Style::Body, label);
                sized(w, s(CHECKBOX).max(lh(Style::Body)))
            }
        }
        Kind::Day { .. } => sized(s(PILL_W), s(PILL_H)),
        Kind::Stepper { .. } | Kind::Time { .. } => {
            let zone = s(STEPPER_ZONE);
            let r = sized(2 * zone + s(STEPPER_VALUE), s(SIZE_FIELD));
            let h = r.bottom - r.top;
            parts.push((Part::Minus, rect(r.left, r.top, zone, h)));
            parts.push((Part::Plus, rect(r.right - zone, r.top, zone, h)));
            parts.push((
                Part::Field,
                rect(r.left + zone, r.top, r.right - r.left - 2 * zone, h),
            ));
            r
        }
        Kind::Segmented { options, .. } => {
            let n = options.len().max(1) as i32;
            let (pad, gap) = (s(SEGMENT_PAD), s(SEGMENT_GAP));
            let seg = match given_w {
                Some(w) => (w - 2 * pad - (n - 1) * gap) / n,
                None => options
                    .iter()
                    .map(|o| m.width(Style::CaptionStrong, o) + 2 * s(SPACE_3))
                    .fold(s(SEGMENT_MIN_W), i32::max),
            };
            let r = sized(2 * pad + n * seg + (n - 1) * gap, s(SIZE_FIELD));
            let seg_h = s(SEGMENT_H);
            let top = r.top + (r.bottom - r.top - seg_h) / 2;
            for k in 0..options.len() {
                let left = r.left + pad + k as i32 * (seg + gap);
                parts.push((Part::Segment(k), rect(left, top, seg, seg_h)));
            }
            r
        }
        Kind::DropDown { entries, other, .. } => {
            let widest = entries
                .iter()
                .map(|e| e.name.as_str())
                .chain([other.as_str()])
                .map(|t| m.width(Style::Body, t))
                .max()
                .unwrap_or(0);
            let w = s(SPACE_3) + widest + s(SPACE_2) + s(CHEVRON) + s(CHEVRON_RIGHT);
            sized(w, s(SIZE_FIELD))
        }
        Kind::Choice {
            title, note, aside, ..
        } => {
            let (aside_w, aside_h) = aside
                .as_ref()
                .map_or((0, 0), |a| (s(SPACE_4) + s(a.w), s(a.h)));
            let chrome = 2 * s(CHOICE_PAD_X) + s(RADIO) + s(SPACE_3) + aside_w;
            // The note wraps to the text column of a card given a width, and at each `\n`.
            lines = match given_w {
                Some(w) => wrap(note, w - chrome, |t| m.width(Style::Caption, t)),
                None => wrap(note, i32::MAX, |t| m.width(Style::Caption, t)),
            };
            let text = lines
                .iter()
                .map(|l| m.width(Style::Caption, l))
                .chain([m.width(Style::BodyStrong, title)])
                .max()
                .unwrap_or(0);
            let notes = lines.len() as i32 * lh(Style::Caption);
            let w = chrome + text;
            let h = 2 * s(CHOICE_PAD_Y) + (lh(Style::BodyStrong) + notes).max(aside_h);
            let r = sized(w, h);
            // Never shorter than its lines: the room past its place is growth.
            rect(r.left, r.top, r.right - r.left, (r.bottom - r.top).max(h))
        }
    };
    if parts.is_empty() {
        parts.push((Part::Body, r));
    }
    Laid {
        rect: r,
        lines,
        parts,
        rows: Vec::new(),
    }
}

/// How far items placed at `y` (96 DPI) move down for the `growth` above them, in pixels.
fn shift(growth: &[(i32, i32)], y: i32) -> i32 {
    growth
        .iter()
        .filter(|&&(under, _)| under <= y)
        .map(|&(_, extra)| extra)
        .sum()
}

/// The open menu's size and rows, in the menu window's client pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct MenuLayout {
    pub w: i32,
    pub h: i32,
    pub rows: Vec<RECT>,
}

impl MenuLayout {
    pub fn hit(&self, x: i32, y: i32) -> Option<usize> {
        self.rows.iter().position(|r| inside(r, x, y))
    }
}

/// Lays out a menu of `entries` at `dpi`, at least `field_w` wide: `MENU_PAD` round the rows,
/// each row padded, the check mark's column, the name and the optional note line.
pub fn menu_layout(entries: &[Entry], field_w: i32, dpi: u32, m: &dyn Measure) -> MenuLayout {
    let s = |v: i32| px(v, dpi);
    let pad = s(MENU_PAD);
    let text_x = s(MENU_ITEM_PAD_X) + s(MENU_CHECK) + s(SPACE_2);
    let widest = entries
        .iter()
        .map(|e| {
            let note = e.note.as_deref().map_or(0, |n| m.width(Style::Caption, n));
            m.width(Style::BodyStrong, &e.name).max(note)
        })
        .max()
        .unwrap_or(0);
    let w = field_w.max(2 * pad + text_x + widest + s(MENU_ITEM_PAD_X));
    let mut y = pad;
    let mut rows = Vec::new();
    for e in entries {
        let note = if e.note.is_some() {
            s(Style::Caption.line_height())
        } else {
            0
        };
        let h = 2 * s(MENU_ITEM_PAD_Y) + s(Style::BodyStrong.line_height()) + note;
        rows.push(rect(pad, y, w - 2 * pad, h));
        y += h;
    }
    MenuLayout {
        w,
        h: y + pad,
        rows,
    }
}

/// Where the menu window goes on screen: `gap` under the `field` (in screen pixels), or above
/// it when there is no room below in the `work` area but there is above. Across, it starts at
/// the field's left edge; a menu wider than its field that would then cross `edge`, the right
/// edge of the form's client area, lines up its right edge with the field's instead, so it does
/// not hang off the window. Kept inside `work` across.
pub fn menu_place(field: RECT, w: i32, h: i32, gap: i32, edge: i32, work: RECT) -> RECT {
    let below = field.bottom + gap;
    let above = field.top - gap - h;
    let top = if below + h > work.bottom && above >= work.top {
        above
    } else {
        below
    };
    let start = if field.left + w > edge {
        field.right - w
    } else {
        field.left
    };
    let left = start.min(work.right - w).max(work.left);
    rect(left, top, w, h)
}

// ---- Scrolling (issue #84) ----

/// How a form taller than its window scrolls. The form is laid out in full, in form pixels;
/// the window shows it in client pixels. When the form fits, the two are the same. When it
/// does not, the window is `view` tall: the content above the footer band moves up by
/// `offset` under the caption, and the footer band stays at the bottom of the window, moved
/// up by the whole [`Scroll::range`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Scroll {
    /// The form's full height and the top of its footer band (`full` when it has none).
    full: i32,
    footer: i32,
    /// The client height.
    view: i32,
    /// How far the content has moved up, from 0 to [`Scroll::range`].
    offset: i32,
    /// Wheel movement that is not a whole pixel yet, in 1/120 pixels.
    rest: i32,
}

impl Scroll {
    /// A form `full` tall with its footer band at `footer`, in a window whose client may be at
    /// most `room` tall. The window takes the full height when it fits. Otherwise it takes
    /// `room`, but never less than the footer band.
    pub fn new(full: i32, footer: Option<i32>, room: i32) -> Scroll {
        let full = full.max(0);
        let footer = footer.unwrap_or(full).clamp(0, full);
        Scroll {
            full,
            footer,
            view: full.min(room).max(full - footer),
            offset: 0,
            rest: 0,
        }
    }

    /// The client height: the form's full height, or less when it does not fit.
    pub fn view(&self) -> i32 {
        self.view
    }

    /// The form's full height.
    pub fn full(&self) -> i32 {
        self.full
    }

    /// How far the content has moved up.
    pub fn offset(&self) -> i32 {
        self.offset
    }

    /// The largest offset: how much of the form does not fit. 0 when it fits.
    pub fn range(&self) -> i32 {
        self.full - self.view
    }

    pub fn scrolls(&self) -> bool {
        self.range() > 0
    }

    /// The client height of the part that scrolls: from the top to the footer band.
    pub fn content(&self) -> i32 {
        self.view - (self.full - self.footer)
    }

    /// How far a rectangle whose top is `top` in form pixels moves up on the client.
    fn lift(&self, top: i32) -> i32 {
        if top >= self.footer {
            self.range()
        } else {
            self.offset
        }
    }

    /// Where `laid`, a rectangle in form pixels such as an item's [`Laid::rect`], is on the
    /// client area now. Part of it may be outside the client area or under the footer band.
    pub fn on_client(&self, laid: RECT) -> RECT {
        let lift = self.lift(laid.top);
        RECT {
            top: laid.top - lift,
            bottom: laid.bottom - lift,
            ..laid
        }
    }

    /// The form pixel at client pixel (`x`, `y`): in the footer band at the bottom, or in the
    /// content that has scrolled.
    pub fn in_form(&self, x: i32, y: i32) -> (i32, i32) {
        if y >= self.content() {
            (x, y + self.range())
        } else {
            (x, y + self.offset)
        }
    }

    /// Whether any of `laid` (form pixels) shows: in the footer band, or in the part of the
    /// content that is scrolled into view.
    pub fn shown(&self, laid: RECT) -> bool {
        let r = self.on_client(laid);
        let (top, bottom) = if laid.top >= self.footer {
            (self.content(), self.view)
        } else {
            (0, self.content())
        };
        r.bottom > top && r.top < bottom && r.bottom > r.top
    }

    /// The part of `laid` (form pixels) that shows, on the client area: cut at the top of the
    /// client and at the footer band, as [`Scroll::shown`] sees it. `None` when none shows.
    /// Screen readers take control bounds from it (#80).
    pub fn visible(&self, laid: RECT) -> Option<RECT> {
        if !self.shown(laid) {
            return None;
        }
        let r = self.on_client(laid);
        let (top, bottom) = if laid.top >= self.footer {
            (self.content(), self.view)
        } else {
            (0, self.content())
        };
        Some(RECT {
            top: r.top.max(top),
            bottom: r.bottom.min(bottom),
            ..r
        })
    }

    /// Moves the content to `offset`, held in range. True when it moved.
    pub fn scroll_to(&mut self, offset: i32) -> bool {
        let next = offset.clamp(0, self.range());
        let moved = next != self.offset;
        self.offset = next;
        moved
    }

    pub fn scroll_by(&mut self, dy: i32) -> bool {
        self.scroll_to(self.offset + dy)
    }

    /// Scrolls as little as needed to show `laid` (form pixels) with `room` above and below,
    /// as when keyboard focus moves to it. A rectangle taller than the content's part of the
    /// window shows its top. Nothing moves for a rectangle in the footer band.
    pub fn reveal(&mut self, laid: RECT, room: i32) -> bool {
        if laid.top >= self.footer {
            return false;
        }
        let (top, bottom) = (laid.top - room, laid.bottom + room);
        let shown = self.content();
        let to = if top < self.offset || bottom - top > shown {
            top
        } else if bottom > self.offset + shown {
            bottom - shown
        } else {
            self.offset
        };
        self.scroll_to(to)
    }

    /// A wheel turn of `delta` (`WHEEL_DELTA`, 120, a notch; more than 0 away from the user,
    /// which scrolls up), at `lines` lines a notch of `line` pixels each, or a page a notch when
    /// `lines` is `WHEEL_PAGESCROLL`. Touchpads send parts of a notch; the parts add up.
    pub fn wheel(&mut self, delta: i32, lines: u32, line: i32) -> bool {
        const NOTCH: i32 = 120;
        let notch = if lines == u32::MAX {
            self.content()
        } else {
            lines.min(100) as i32 * line
        };
        self.rest -= delta * notch;
        let step = self.rest / NOTCH;
        self.rest -= step * NOTCH;
        let moved = step != 0 && self.scroll_by(step);
        if !moved && step != 0 {
            // At an end: what is left of the turn would only delay the way back.
            self.rest = 0;
        }
        moved
    }

    /// Where the thumb can run, as (top, length) in client pixels at `dpi`: the content's part
    /// of the window, `scrollbar-inset` in from its ends.
    fn track(&self, dpi: u32) -> (i32, i32) {
        let inset = px(SCROLLBAR_INSET, dpi);
        (inset, (self.content() - 2 * inset).max(0))
    }

    /// The thumb as (top, length) in client pixels: as long against its track as the content
    /// shown is against all of it, never shorter than `scrollbar-min`.
    fn thumb_span(&self, dpi: u32) -> (i32, i32) {
        let (top, len) = self.track(dpi);
        let shown = self.content().max(0);
        let long = if self.footer > 0 {
            (len * shown / self.footer)
                .max(px(SCROLLBAR_MIN, dpi))
                .min(len)
        } else {
            len
        };
        let free = len - long;
        let at = if self.range() > 0 {
            free * self.offset / self.range()
        } else {
            0
        };
        (top + at, long)
    }

    /// The scrollbar's thumb on a client `w` wide at `dpi`, `hot` from 0 (at rest,
    /// `scrollbar-w` wide) to 1 (under the pointer, `scrollbar-hot-w`). `None` when nothing
    /// scrolls.
    pub fn thumb(&self, w: i32, dpi: u32, hot: f32) -> Option<Box2> {
        if !self.scrolls() {
            return None;
        }
        let (rest, wide) = (px(SCROLLBAR_W, dpi) as f32, px(SCROLLBAR_HOT_W, dpi) as f32);
        let thick = rest + (wide - rest) * hot.clamp(0.0, 1.0);
        let (top, long) = self.thumb_span(dpi);
        Some(Box2 {
            x: (w - px(SCROLLBAR_INSET, dpi)) as f32 - thick,
            y: top as f32,
            w: thick,
            h: long as f32,
        })
    }

    /// The strip at the right of the content's part of a client `w` wide that answers the
    /// pointer: the hot thumb's width and its inset each side. `None` when nothing scrolls.
    pub fn bar(&self, w: i32, dpi: u32) -> Option<RECT> {
        let wide = px(SCROLLBAR_HOT_W, dpi) + 2 * px(SCROLLBAR_INSET, dpi);
        self.scrolls()
            .then(|| rect(w - wide, 0, wide, self.content()))
    }

    /// Whether client `y` is on the thumb, across its track.
    pub fn on_thumb(&self, y: i32, dpi: u32) -> bool {
        let (top, long) = self.thumb_span(dpi);
        y >= top && y < top + long
    }

    /// Drags the thumb: the offset when the drag began at `from`, the pointer since moved by
    /// `dy` client pixels.
    pub fn drag(&mut self, from: i32, dy: i32, dpi: u32) -> bool {
        let (_, len) = self.track(dpi);
        let free = len - self.thumb_span(dpi).1;
        if free <= 0 {
            return false;
        }
        let moved = (i64::from(dy) * i64::from(self.range()) / i64::from(free)) as i32;
        self.scroll_to(from + moved)
    }

    /// A click on the track off the thumb at client `y`: a page toward it.
    pub fn page_toward(&mut self, y: i32, dpi: u32) -> bool {
        let (top, long) = self.thumb_span(dpi);
        if y < top {
            self.scroll_by(-self.content())
        } else if y >= top + long {
            self.scroll_by(self.content())
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Aside, Entry, Ink, Place};
    use super::*;
    use std::rc::Rc;

    /// Every character 7px wide at 96 DPI, scaled with the DPI, at any style.
    struct Fixed(u32);
    impl Measure for Fixed {
        fn width(&self, _: Style, text: &str) -> i32 {
            px(7 * text.chars().count() as i32, self.0)
        }
    }

    const SAVE: Id = 1;
    const CANCEL: Id = 2;
    const SWITCH: Id = 3;
    const CHECK: Id = 4;
    const DAY_MON: Id = 5;
    const EVERY: Id = 6;
    const FROM: Id = 7;
    const SOFT_FIRM: Id = 8;
    const PROFILE: Id = 9;
    const CARD_A: Id = 10;
    const CARD_B: Id = 11;
    const CARD_C: Id = 12;
    const LABEL: Id = 13;
    const CARD: Id = 14;

    fn entries() -> Vec<Entry> {
        vec![
            Entry::new("Gentle", Some("Every 45 min")),
            Entry::new("Balanced", Some("Every 30 min")),
            Entry::new("Strict", None),
        ]
    }

    fn choice(group: u16, title: &str, selected: bool) -> Kind {
        Kind::Choice {
            group,
            title: title.to_owned(),
            note: "numbers".to_owned(),
            selected,
            aside: None,
        }
    }

    /// A small form with one of everything, in this list order.
    fn sample() -> Form {
        let items = vec![
            Item::new(
                CARD,
                Place::new(0, 0, 400, 300),
                Kind::Card { rows: vec![40] },
            ),
            Item::new(
                LABEL,
                Place::at(10, 10),
                Kind::text("Work days", Style::Body, Ink::Main),
            ),
            Item::new(
                CARD_A,
                Place::new(10, 40, 300, 0),
                choice(1, "Gentle", false),
            ),
            Item::new(
                CARD_B,
                Place::new(10, 110, 300, 0),
                choice(1, "Balanced", true),
            ),
            Item::new(
                CARD_C,
                Place::new(10, 180, 300, 0),
                choice(1, "Strict", false),
            ),
            Item::new(
                DAY_MON,
                Place::at(10, 260),
                Kind::Day {
                    label: "Mon".into(),
                    on: false,
                },
            ),
            Item::new(SWITCH, Place::at(80, 260), Kind::Switch { on: false }),
            Item::new(
                CHECK,
                Place::at(140, 260),
                Kind::Checkbox {
                    on: true,
                    label: "Start".into(),
                },
            ),
            Item::new(
                EVERY,
                Place::at(10, 310),
                Kind::Stepper {
                    value: 30,
                    min: 10,
                    max: 240,
                    step: 1,
                    unit: "min".into(),
                },
            ),
            Item::new(
                FROM,
                Place::at(140, 310),
                Kind::Time {
                    minutes: 8 * 60,
                    step: 15,
                },
            ),
            Item::new(
                SOFT_FIRM,
                Place::at(260, 310),
                Kind::Segmented {
                    options: vec!["Soft".into(), "Firm".into()],
                    selected: 0,
                },
            ),
            Item::new(
                PROFILE,
                Place::new(10, 360, 176, 0),
                Kind::DropDown {
                    entries: entries(),
                    current: Some(1),
                    other: "Custom".into(),
                },
            ),
            Item::new(
                SAVE,
                Place::at(200, 420),
                Kind::Button {
                    label: "Save".into(),
                    kind: ButtonKind::Primary,
                    default: true,
                    cancel: false,
                },
            ),
            Item::new(
                CANCEL,
                Place::at(300, 420),
                Kind::Button {
                    label: "Cancel".into(),
                    kind: ButtonKind::Secondary,
                    default: false,
                    cancel: true,
                },
            ),
        ];
        let mut form = Form::new(items);
        form.lay_out(96, &Fixed(96));
        form.focus_in();
        form
    }

    fn focus_after(form: &mut Form, keys: &[Key]) -> Option<Id> {
        for &k in keys {
            form.key(k);
        }
        form.focused()
    }

    fn centre(form: &Form, id: Id, part: Part) -> (i32, i32) {
        let laid = &form.laid()[form.index(id).unwrap()];
        let r = laid.parts.iter().find(|(p, _)| *p == part).unwrap().1;
        ((r.left + r.right) / 2, (r.top + r.bottom) / 2)
    }

    fn click(form: &mut Form, id: Id, part: Part) -> Vec<Event> {
        let (x, y) = centre(form, id, part);
        let mut out = form.pointer_down(x, y);
        out.extend(form.pointer_up(x, y));
        out
    }

    // ---- Focus order ----

    #[test]
    fn tab_visits_the_tab_stops_in_list_order_and_wraps() {
        let mut f = sample();
        // The card group is one stop, at its selected card; text and cards are skipped.
        assert_eq!(f.focused(), Some(CARD_B));
        let order: Vec<_> = (0..10)
            .map(|_| focus_after(&mut f, &[Key::Tab]).unwrap())
            .collect();
        assert_eq!(
            order,
            [
                DAY_MON, SWITCH, CHECK, EVERY, FROM, SOFT_FIRM, PROFILE, SAVE, CANCEL, CARD_B
            ]
        );
    }

    #[test]
    fn shift_tab_goes_back_and_wraps() {
        let mut f = sample();
        assert_eq!(focus_after(&mut f, &[Key::BackTab]), Some(CANCEL));
        assert_eq!(focus_after(&mut f, &[Key::BackTab]), Some(SAVE));
        assert_eq!(focus_after(&mut f, &[Key::Tab, Key::Tab]), Some(CARD_B));
    }

    #[test]
    fn a_group_with_no_selection_stops_at_its_first_card() {
        let mut f = sample();
        f.set(CARD_B, Value::On(false));
        assert!(f.tab_stops().contains(&f.index(CARD_A).unwrap()));
        assert!(!f.tab_stops().contains(&f.index(CARD_B).unwrap()));
    }

    #[test]
    fn the_ring_shows_only_after_keyboard_use_and_while_focused() {
        let mut f = sample();
        let ringed = |f: &Form| f.looks().iter().filter(|l| l.ring).count();
        assert_eq!(ringed(&f), 0);
        click(&mut f, SWITCH, Part::Body);
        assert_eq!(ringed(&f), 0, "a click does not show the ring");
        f.key(Key::Tab);
        assert_eq!(ringed(&f), 1);
        assert!(f.looks()[f.index(CHECK).unwrap()].ring);
        f.focus_out();
        assert_eq!(ringed(&f), 0);
        f.focus_in();
        assert_eq!(ringed(&f), 1);
    }

    // ---- Enter, Esc, Space ----

    #[test]
    fn enter_presses_the_default_button_and_esc_the_cancel() {
        let mut f = sample();
        assert_eq!(f.key(Key::Enter), [Event::Pressed(SAVE)]);
        assert_eq!(f.key(Key::Esc), [Event::Pressed(CANCEL)]);
        assert_eq!(f.escape(), Event::Pressed(CANCEL));
    }

    #[test]
    fn enter_presses_the_focused_button_as_in_a_windows_dialog() {
        let mut f = sample();
        f.focus(CANCEL);
        assert_eq!(f.key(Key::Enter), [Event::Pressed(CANCEL)]);
        f.focus(SAVE);
        assert_eq!(f.key(Key::Enter), [Event::Pressed(SAVE)]);
        // Any other control: the default button.
        f.focus(SWITCH);
        assert_eq!(f.key(Key::Enter), [Event::Pressed(SAVE)]);
    }

    #[test]
    fn focus_from_code_shows_the_ring() {
        let mut f = sample();
        // A click on Save hides nothing and shows nothing: no keyboard yet.
        click(&mut f, SAVE, Part::Body);
        assert!(!f.looks().iter().any(|l| l.ring));
        // The refused Save focuses its field, and the user must see where.
        f.focus(EVERY);
        assert!(f.looks()[f.index(EVERY).unwrap()].ring);
    }

    #[test]
    fn esc_without_a_cancel_button_asks_to_close() {
        let mut f = Form::new(vec![Item::new(
            1,
            Place::at(0, 0),
            Kind::Switch { on: false },
        )]);
        f.lay_out(96, &Fixed(96));
        assert_eq!(f.key(Key::Esc), [Event::Close]);
        assert_eq!(f.key(Key::Enter), []);
    }

    #[test]
    fn one_button_can_be_default_and_cancel() {
        let mut f = Form::new(vec![Item::new(
            1,
            Place::at(0, 0),
            Kind::Button {
                label: "Start".into(),
                kind: ButtonKind::Primary,
                default: true,
                cancel: true,
            },
        )]);
        assert_eq!(f.key(Key::Enter), [Event::Pressed(1)]);
        assert_eq!(f.key(Key::Esc), [Event::Pressed(1)]);
    }

    #[test]
    fn space_toggles_switches_checkboxes_and_days_and_presses_buttons() {
        let mut f = sample();
        f.focus(SWITCH);
        assert_eq!(f.key(Key::Space), [Event::Changed(SWITCH, Value::On(true))]);
        assert_eq!(
            f.key(Key::Space),
            [Event::Changed(SWITCH, Value::On(false))]
        );
        f.focus(CHECK);
        assert_eq!(f.key(Key::Space), [Event::Changed(CHECK, Value::On(false))]);
        f.focus(DAY_MON);
        assert_eq!(
            f.key(Key::Space),
            [Event::Changed(DAY_MON, Value::On(true))]
        );
        assert_eq!(f.value(DAY_MON), Some(Value::On(true)));
        f.focus(CANCEL);
        assert_eq!(f.key(Key::Space), [Event::Pressed(CANCEL)]);
    }

    // ---- Segmented and selectable cards ----

    #[test]
    fn arrows_move_the_segmented_selection_and_wrap() {
        let mut f = sample();
        f.focus(SOFT_FIRM);
        let firm = Event::Changed(SOFT_FIRM, Value::Index(Some(1)));
        let soft = Event::Changed(SOFT_FIRM, Value::Index(Some(0)));
        assert_eq!(f.key(Key::Right), [firm]);
        assert_eq!(f.key(Key::Right), [soft]);
        assert_eq!(f.key(Key::Left), [firm]);
        assert_eq!(f.key(Key::Up), [soft]);
        assert_eq!(f.key(Key::Down), [firm]);
        assert_eq!(f.focused(), Some(SOFT_FIRM));
    }

    #[test]
    fn arrows_move_the_card_selection_and_the_focus_with_it() {
        let mut f = sample();
        assert_eq!(f.key(Key::Down), [Event::Changed(CARD_C, Value::On(true))]);
        assert_eq!(f.focused(), Some(CARD_C));
        assert_eq!(f.value(CARD_B), Some(Value::On(false)));
        assert_eq!(f.key(Key::Right), [Event::Changed(CARD_A, Value::On(true))]);
        assert_eq!(f.key(Key::Up), [Event::Changed(CARD_C, Value::On(true))]);
        assert_eq!(f.key(Key::Left), [Event::Changed(CARD_B, Value::On(true))]);
        // Still one stop: Tab leaves the group.
        assert_eq!(focus_after(&mut f, &[Key::Tab]), Some(DAY_MON));
    }

    // ---- Steppers ----

    #[test]
    fn stepper_keys_step_page_and_jump_to_the_ends() {
        let mut f = sample();
        f.focus(EVERY);
        let n = |v| [Event::Changed(EVERY, Value::Number(v))];
        assert_eq!(f.key(Key::Up), n(31));
        assert_eq!(f.key(Key::Down), n(30));
        assert_eq!(f.key(Key::PageUp), n(35));
        assert_eq!(f.key(Key::PageDown), n(30));
        assert_eq!(f.key(Key::End), n(240));
        assert_eq!(f.key(Key::Up), [], "held at max");
        assert_eq!(f.key(Key::Home), n(10));
        assert_eq!(f.key(Key::Down), [], "held at min");
        assert_eq!(f.key(Key::Left), []);
    }

    #[test]
    fn stepping_snaps_to_the_step_and_clamps() {
        assert_eq!(stepped(7, 15, 1, 0, 100), 15);
        assert_eq!(stepped(7, 15, -1, 0, 100), 0);
        assert_eq!(stepped(15, 15, 1, 0, 100), 30);
        assert_eq!(stepped(15, 15, -1, 0, 100), 0);
        assert_eq!(stepped(95, 15, 1, 0, 100), 100);
        assert_eq!(stepped(30, 1, 5, 10, 240), 35);
        // A value typed out of range comes back into it on the first step.
        assert_eq!(stepped(999, 1, 1, 10, 240), 240);
        assert_eq!(stepped(999, 1, -1, 10, 240), 240);
        assert_eq!(stepped(0, 1, 1, 10, 240), 10);
        assert_eq!(stepped(0, 1, -1, 10, 240), 10);
    }

    #[test]
    fn time_steps_by_quarter_hours_and_wraps_round_midnight() {
        assert_eq!(stepped_time(8 * 60, 15, 1), 8 * 60 + 15);
        assert_eq!(stepped_time(8 * 60 + 7, 15, 1), 8 * 60 + 15);
        assert_eq!(stepped_time(8 * 60 + 7, 15, -1), 8 * 60);
        assert_eq!(stepped_time(23 * 60 + 45, 15, 1), 0);
        assert_eq!(stepped_time(0, 15, -1), 23 * 60 + 45);
        assert_eq!(stepped_time(0, 15, -5), 22 * 60 + 45);
        let mut f = sample();
        f.focus(FROM);
        assert_eq!(f.key(Key::Home), [Event::Changed(FROM, Value::Number(0))]);
        assert_eq!(
            f.key(Key::End),
            [Event::Changed(FROM, Value::Number(DAY - 1))]
        );
        assert_eq!(time_text(DAY - 1), "23:59");
        assert_eq!(time_text(8 * 60 + 5), "08:05");
    }

    #[test]
    fn typed_digits_take_effect_at_once_and_enter_presses_the_default() {
        let mut f = sample();
        f.focus(EVERY);
        assert_eq!(
            f.key(Key::Char('4')),
            [Event::Changed(EVERY, Value::Number(4))]
        );
        assert_eq!(
            f.key(Key::Char('5')),
            [Event::Changed(EVERY, Value::Number(45))]
        );
        assert_eq!(
            f.looks()[f.index(EVERY).unwrap()].edit.as_deref(),
            Some("45")
        );
        assert_eq!(f.value(EVERY), Some(Value::Number(45)));
        assert_eq!(f.key(Key::Enter), [Event::Pressed(SAVE)]);
        assert_eq!(f.looks()[f.index(EVERY).unwrap()].edit, None);
    }

    #[test]
    fn typing_ends_on_tab_focus_loss_and_click_and_no_number_gives_the_old_one_back() {
        let mut f = sample();
        f.focus(EVERY);
        f.key(Key::Char('5'));
        f.key(Key::Char('0'));
        assert_eq!(f.key(Key::Tab), []);
        assert_eq!(f.focused(), Some(FROM));
        assert_eq!(f.value(EVERY), Some(Value::Number(50)));
        f.focus(EVERY);
        f.key(Key::Char('6'));
        assert_eq!(f.focus_out(), []);
        assert_eq!(f.looks()[f.index(EVERY).unwrap()].edit, None);
        // A click that moves the focus ends it too.
        f.focus_in();
        f.key(Key::Char('7'));
        assert_eq!(
            click(&mut f, SWITCH, Part::Body),
            [Event::Changed(SWITCH, Value::On(true))]
        );
        // Typed down to nothing: the value from before the typing comes back.
        f.focus(EVERY);
        f.key(Key::Back);
        assert_eq!(f.value(EVERY), Some(Value::Number(7)));
        assert_eq!(f.key(Key::Tab), []);
        f.focus(EVERY);
        assert_eq!(
            f.key(Key::Char('9')),
            [Event::Changed(EVERY, Value::Number(9))]
        );
        f.key(Key::Back);
        assert_eq!(f.key(Key::Tab), [Event::Changed(EVERY, Value::Number(7))]);
    }

    #[test]
    fn a_typed_value_out_of_range_stays_as_typed() {
        let mut f = sample();
        f.focus(EVERY);
        for c in "999".chars() {
            f.key(Key::Char(c));
        }
        f.key(Key::Tab);
        assert_eq!(f.value(EVERY), Some(Value::Number(999)));
        f.focus(EVERY);
        f.key(Key::Char('0'));
        f.key(Key::Tab);
        assert_eq!(f.value(EVERY), Some(Value::Number(0)));
        // The next step brings it back into range.
        f.focus(EVERY);
        assert_eq!(f.key(Key::Up), [Event::Changed(EVERY, Value::Number(10))]);
    }

    #[test]
    fn typing_keeps_digits_only_up_to_the_limit_and_backspace_edits() {
        let mut f = sample();
        f.focus(EVERY);
        for c in "12a:345".chars() {
            f.key(Key::Char(c));
        }
        let edit = |f: &Form| f.looks()[f.index(EVERY).unwrap()].edit.clone();
        assert_eq!(edit(&f).as_deref(), Some("1234"));
        f.key(Key::Back);
        assert_eq!(edit(&f).as_deref(), Some("123"));
        assert_eq!(f.value(EVERY), Some(Value::Number(123)));
        // One Esc cancels, also while typing; the typed value goes back first.
        assert_eq!(
            f.key(Key::Esc),
            [
                Event::Changed(EVERY, Value::Number(30)),
                Event::Pressed(CANCEL)
            ]
        );
        assert_eq!(edit(&f), None);
        assert_eq!(f.value(EVERY), Some(Value::Number(30)));
        // Backspace with nothing typed edits the value shown.
        f.key(Key::Back);
        assert_eq!(edit(&f).as_deref(), Some("3"));
        // Nothing typed at all: the value from before comes back.
        f.key(Key::Back);
        assert_eq!(f.key(Key::Tab), [Event::Changed(EVERY, Value::Number(30))]);
        assert_eq!(f.value(EVERY), Some(Value::Number(30)));
    }

    #[test]
    fn up_while_typing_steps_from_the_typed_value() {
        let mut f = sample();
        f.focus(EVERY);
        f.key(Key::Char('4'));
        f.key(Key::Char('0'));
        assert_eq!(f.key(Key::Up), [Event::Changed(EVERY, Value::Number(41))]);
    }

    #[test]
    fn typed_times_take_several_forms_and_bad_ones_revert() {
        for (typed, want) in [
            ("8", Some(480)),
            ("08", Some(480)),
            ("830", Some(510)),
            ("0830", Some(510)),
            ("8:30", Some(510)),
            ("17:05", Some(1025)),
            ("2359", Some(1439)),
            ("2400", None),
            ("0860", None),
            ("", None),
            (":", None),
            ("12345", None),
        ] {
            assert_eq!(parse_time(typed), want, "{typed:?}");
        }
        let mut f = sample();
        f.focus(FROM);
        for c in "9:15".chars() {
            f.key(Key::Char(c));
        }
        assert_eq!(f.key(Key::Tab), [Event::Changed(FROM, Value::Number(555))]);
        f.focus(FROM);
        for c in "99".chars() {
            f.key(Key::Char(c));
        }
        assert_eq!(f.key(Key::Tab), []);
        assert_eq!(f.value(FROM), Some(Value::Number(555)));
    }

    // ---- Drop-down ----

    #[test]
    fn space_and_alt_down_open_the_menu() {
        for open in [Key::Space, Key::AltDown] {
            let mut f = sample();
            f.focus(PROFILE);
            assert_eq!(f.key(open), []);
            let menu = f.menu().expect("open");
            assert_eq!(menu.item, f.index(PROFILE).unwrap());
            assert_eq!(menu.hot, Some(1), "starts on the current entry");
            assert!(f.looks()[menu.item].open);
        }
    }

    #[test]
    fn arrows_move_in_the_menu_and_enter_picks() {
        let mut f = sample();
        f.focus(PROFILE);
        f.key(Key::Space);
        f.key(Key::Down);
        f.key(Key::Down);
        assert_eq!(f.menu().unwrap().hot, Some(2), "held at the last");
        f.key(Key::Up);
        f.key(Key::Up);
        f.key(Key::Up);
        assert_eq!(f.menu().unwrap().hot, Some(0), "held at the first");
        assert_eq!(
            f.key(Key::Enter),
            [Event::Changed(PROFILE, Value::Index(Some(0)))]
        );
        assert_eq!(f.menu(), None);
        assert_eq!(f.value(PROFILE), Some(Value::Index(Some(0))));
        // Picking the current entry changes nothing.
        f.key(Key::Space);
        assert_eq!(f.key(Key::Enter), []);
        // On the closed drop-down, Enter presses the default button.
        assert_eq!(f.key(Key::Enter), [Event::Pressed(SAVE)]);
        assert_eq!(f.menu(), None);
    }

    #[test]
    fn esc_closes_the_menu_without_cancelling() {
        let mut f = sample();
        f.focus(PROFILE);
        f.key(Key::AltDown);
        f.key(Key::Down);
        assert_eq!(f.key(Key::Esc), []);
        assert_eq!(f.menu(), None);
        assert_eq!(f.value(PROFILE), Some(Value::Index(Some(1))));
        assert_eq!(f.key(Key::Esc), [Event::Pressed(CANCEL)]);
    }

    #[test]
    fn a_click_outside_closes_the_menu_and_does_nothing_else() {
        let mut f = sample();
        click(&mut f, PROFILE, Part::Body);
        assert!(f.menu().is_some(), "a click on the field opens it");
        assert_eq!(click(&mut f, SWITCH, Part::Body), []);
        assert_eq!(f.menu(), None);
        assert_eq!(f.value(SWITCH), Some(Value::On(false)));
        // A click on the field while open closes it too.
        click(&mut f, PROFILE, Part::Body);
        click(&mut f, PROFILE, Part::Body);
        assert_eq!(f.menu(), None);
    }

    #[test]
    fn the_menu_picks_by_pointer_and_shows_other_when_nothing_matches() {
        let mut f = sample();
        f.set(PROFILE, Value::Index(None));
        click(&mut f, PROFILE, Part::Body);
        assert_eq!(f.menu().unwrap().hot, Some(0));
        assert!(f.menu_hover(Some(2)));
        assert!(!f.menu_hover(None), "leaving keeps the last highlight");
        assert_eq!(
            f.menu_pick(2),
            [Event::Changed(PROFILE, Value::Index(Some(2)))]
        );
        assert!(!f.set(PROFILE, Value::Index(Some(3))), "no such entry");
    }

    #[test]
    fn focus_loss_and_tab_close_the_menu() {
        let mut f = sample();
        f.focus(PROFILE);
        f.key(Key::Space);
        f.focus_out();
        assert_eq!(f.menu(), None);
        f.focus_in();
        f.key(Key::Space);
        assert_eq!(f.key(Key::Tab), []);
        assert_eq!(f.menu(), None);
        assert_eq!(f.focused(), Some(SAVE));
    }

    #[test]
    fn menu_layout_and_place() {
        let m = menu_layout(&entries(), 176, 96, &Fixed(96));
        // Rows: 8 + 20 + 18 + 8 with a note, 8 + 20 + 8 without; 4 round them.
        assert_eq!(m.rows[0], rect(4, 4, 168, 54));
        assert_eq!(m.rows[1], rect(4, 58, 168, 54));
        assert_eq!(m.rows[2], rect(4, 112, 168, 36));
        assert_eq!((m.w, m.h), (176, 152));
        assert_eq!(m.hit(10, 60), Some(1));
        assert_eq!(m.hit(1, 60), None);

        let work = rect(0, 0, 1000, 800);
        let edge = 1000;
        let field = rect(100, 300, 176, 32);
        assert_eq!(
            menu_place(field, 176, 152, 4, edge, work),
            rect(100, 336, 176, 152)
        );
        // No room below: above.
        let low = rect(100, 700, 176, 32);
        assert_eq!(
            menu_place(low, 176, 152, 4, edge, work),
            rect(100, 544, 176, 152)
        );
        // Kept inside the work area across.
        let right = rect(900, 300, 176, 32);
        assert_eq!(menu_place(right, 176, 152, 4, 2000, work).left, 1000 - 176);
        // Wider than its field near the window's right edge: right edges line up.
        let near = rect(384, 300, 176, 32);
        assert_eq!(menu_place(near, 330, 152, 4, 600, work).left, 560 - 330);
        assert_eq!(menu_place(near, 330, 152, 4, 800, work).left, 384);
    }

    // ---- Pointer ----

    #[test]
    fn clicks_act_on_release_over_the_part_the_press_started_on() {
        let mut f = sample();
        assert_eq!(click(&mut f, SAVE, Part::Body), [Event::Pressed(SAVE)]);
        assert_eq!(f.focused(), Some(SAVE));
        // Pressed on Save, released on Cancel: nothing.
        let (x, y) = centre(&f, SAVE, Part::Body);
        f.pointer_down(x, y);
        let (x, y) = centre(&f, CANCEL, Part::Body);
        assert_eq!(f.pointer_up(x, y), []);
        assert_eq!(
            click(&mut f, CHECK, Part::Body),
            [Event::Changed(CHECK, Value::On(false))]
        );
        assert_eq!(
            click(&mut f, DAY_MON, Part::Body),
            [Event::Changed(DAY_MON, Value::On(true))]
        );
        assert_eq!(
            click(&mut f, SOFT_FIRM, Part::Segment(1)),
            [Event::Changed(SOFT_FIRM, Value::Index(Some(1)))]
        );
        assert_eq!(click(&mut f, SOFT_FIRM, Part::Segment(1)), []);
        assert_eq!(
            click(&mut f, CARD_A, Part::Body),
            [Event::Changed(CARD_A, Value::On(true))]
        );
        assert_eq!(f.focused(), Some(CARD_A));
        assert_eq!(f.value(CARD_B), Some(Value::On(false)));
    }

    #[test]
    fn a_press_dragged_off_an_unselected_card_leaves_the_focus_alone() {
        let mut f = sample();
        f.focus(SWITCH);
        let (x, y) = centre(&f, CARD_A, Part::Body);
        f.pointer_down(x, y);
        assert_eq!(f.focused(), Some(SWITCH));
        let (x, y) = centre(&f, DAY_MON, Part::Body);
        assert_eq!(f.pointer_up(x, y), []);
        assert_eq!(f.focused(), Some(SWITCH));
        assert_eq!(f.value(CARD_A), Some(Value::On(false)));
        // A press on the selected card focuses it at once, as on any control.
        let (x, y) = centre(&f, CARD_B, Part::Body);
        f.pointer_down(x, y);
        assert_eq!(f.focused(), Some(CARD_B));
    }

    #[test]
    fn tab_leaves_the_group_from_any_of_its_cards() {
        let mut f = sample();
        f.focus(CARD_A);
        assert_eq!(focus_after(&mut f, &[Key::Tab]), Some(DAY_MON));
        f.focus(CARD_C);
        assert_eq!(focus_after(&mut f, &[Key::BackTab]), Some(CANCEL));
        f.focus(CARD_A);
        assert_eq!(focus_after(&mut f, &[Key::BackTab]), Some(CANCEL));
    }

    #[test]
    fn stepper_zones_step_on_press() {
        let mut f = sample();
        let (x, y) = centre(&f, EVERY, Part::Plus);
        assert_eq!(
            f.pointer_down(x, y),
            [Event::Changed(EVERY, Value::Number(31))]
        );
        assert_eq!(f.pointer_up(x, y), []);
        assert_eq!(
            click(&mut f, EVERY, Part::Minus),
            [Event::Changed(EVERY, Value::Number(30))]
        );
        assert_eq!(click(&mut f, EVERY, Part::Field), []);
        assert_eq!(f.focused(), Some(EVERY));
    }

    #[test]
    fn text_and_cards_take_no_clicks() {
        let f = sample();
        let r = f.laid()[f.index(LABEL).unwrap()].rect;
        assert_eq!(f.hit(r.left, r.top), None);
        assert_eq!(f.hit(395, 5), None, "the card under everything");
    }

    // ---- Motion ----

    #[test]
    fn hover_and_press_follow_the_design_timings() {
        let mut f = sample();
        let i = f.index(CANCEL).unwrap();
        let level = |f: &Form| f.looks()[i].levels[0];
        let (x, y) = centre(&f, CANCEL, Part::Body);
        assert!(f.pointer_move(x, y, false));
        assert!(f.moving());
        f.tick(Duration::from_millis(75), true);
        assert!((level(&f) - 0.5).abs() < 1e-3);
        f.tick(Duration::from_millis(75), true);
        assert_eq!(level(&f), HOVER);
        assert!(!f.moving());
        f.pointer_down(x, y);
        f.tick(Duration::from_millis(90), true);
        assert_eq!(level(&f), PRESSED);
        f.pointer_up(x, y);
        f.pointer_leave(false);
        f.tick(Duration::from_millis(90), true);
        assert_eq!(level(&f), HOVER);
        assert!(!f.tick(Duration::from_millis(120), true));
        assert_eq!(level(&f), REST);
    }

    #[test]
    fn nothing_moves_when_animation_is_off() {
        let mut f = sample();
        let (x, y) = centre(&f, SWITCH, Part::Body);
        f.pointer_move(x, y, false);
        f.pointer_down(x, y);
        f.pointer_up(x, y);
        assert!(!f.tick(Duration::from_millis(1), false));
        let look = &f.looks()[f.index(SWITCH).unwrap()];
        assert_eq!((look.levels[0], look.slide), (HOVER, 1.0));
    }

    #[test]
    fn the_knob_slides_in_150_ms() {
        let mut f = sample();
        f.focus(SWITCH);
        f.key(Key::Space);
        let i = f.index(SWITCH).unwrap();
        f.tick(Duration::from_millis(75), true);
        assert!((f.looks()[i].slide - 0.5).abs() < 1e-3);
        assert!(!f.tick(Duration::from_millis(75), true));
        assert_eq!(f.looks()[i].slide, 1.0);
    }

    #[test]
    fn stepper_zones_hover_on_their_own() {
        let mut f = sample();
        let i = f.index(EVERY).unwrap();
        let (x, y) = centre(&f, EVERY, Part::Plus);
        f.pointer_move(x, y, false);
        f.tick(Duration::from_secs(1), true);
        assert_eq!(f.looks()[i].levels, [REST, HOVER, REST]);
    }

    // ---- Layout ----

    #[test]
    fn controls_take_the_table_sizes() {
        let f = sample();
        let size = |id| {
            let r = f.laid()[f.index(id).unwrap()].rect;
            (r.right - r.left, r.bottom - r.top)
        };
        assert_eq!(size(SWITCH), (40, 22));
        assert_eq!(size(DAY_MON), (52, 32));
        assert_eq!(size(EVERY), (112, 32));
        assert_eq!(size(FROM), (112, 32));
        assert_eq!(size(PROFILE), (176, 32));
        // Checkbox: 18 square, 8, the label; as tall as a body line.
        assert_eq!(size(CHECK), (18 + 8 + 35, 20));
        // Segmented: two equal segments, each the wider label and 12 each side (at least 50),
        // padding 2, gap 2.
        assert_eq!(size(SOFT_FIRM), (2 + 52 + 2 + 52 + 2, 32));
        // Buttons: label, 12 + 1 each side; the primary at least 80.
        assert_eq!(size(SAVE), (80, 32));
        assert_eq!(size(CANCEL), (42 + 26, 32));
        // Selectable card: 14 + 20 + 18 + 14.
        assert_eq!(size(CARD_B), (300, 66));
        let parts = &f.laid()[f.index(EVERY).unwrap()].parts;
        assert_eq!(parts[0], (Part::Minus, rect(10, 310, 28, 32)));
        assert_eq!(parts[1], (Part::Plus, rect(10 + 84, 310, 28, 32)));
        assert_eq!(parts[2], (Part::Field, rect(38, 310, 56, 32)));
        let seg = &f.laid()[f.index(SOFT_FIRM).unwrap()].parts;
        assert_eq!(
            seg[1],
            (Part::Segment(1), rect(260 + 2 + 52 + 2, 312, 52, 28))
        );
    }

    #[test]
    fn layout_scales_with_the_dpi() {
        let mut f = sample();
        for dpi in [96, 120, 144, 192] {
            f.lay_out(dpi, &Fixed(dpi));
            let laid = |id| f.laid()[f.index(id).unwrap()].clone();
            assert_eq!(
                laid(SWITCH).rect,
                rect(px(80, dpi), px(260, dpi), px(40, dpi), px(22, dpi))
            );
            assert_eq!(laid(CARD).rect, rect(0, 0, px(400, dpi), px(300, dpi)));
            let every = laid(EVERY);
            assert_eq!(every.parts[0].1.right - every.parts[0].1.left, px(28, dpi));
            assert_eq!(every.rect.bottom - every.rect.top, px(32, dpi));
            let card = laid(CARD_B).rect;
            assert_eq!(
                card.bottom - card.top,
                2 * px(14, dpi) + px(20, dpi) + px(18, dpi)
            );
            // Hit-testing follows: the switch's centre at this DPI is the switch.
            let (x, y) = centre(&f, SWITCH, Part::Body);
            assert_eq!(f.hit(x, y), Some((f.index(SWITCH).unwrap(), Part::Body)));
        }
    }

    #[test]
    fn text_wraps_to_the_width_it_is_given() {
        let mut f = Form::new(vec![Item::new(
            1,
            Place::new(0, 0, 60, 0),
            Kind::text("aaaa bbbb cccc", Style::Caption, Ink::Muted),
        )]);
        f.lay_out(96, &Fixed(96));
        assert_eq!(f.laid()[0].lines, ["aaaa", "bbbb", "cccc"]);
        assert_eq!(f.laid()[0].rect, rect(0, 0, 60, 3 * 18));
    }

    #[test]
    fn wrapped_text_that_needs_more_room_pushes_what_is_under_it_down() {
        // The note reserves one caption line at 96 DPI but needs three.
        let note = Kind::text("aaaa bbbb cccc", Style::Caption, Ink::Muted);
        let switch = Kind::Switch { on: false };
        let mut f = Form::new(vec![
            Item::new(
                1,
                Place::new(0, 0, 200, 100),
                Kind::Card { rows: vec![20, 60] },
            ),
            Item::new(2, Place::new(10, 20, 60, 18), note),
            Item::new(3, Place::at(100, 20), switch.clone()),
            Item::new(4, Place::at(10, 38), switch),
            Item::new(5, Place::new(0, 100, 200, 40), Kind::Band),
        ]);
        f.lay_out(96, &Fixed(96));
        let extra = 3 * 18 - 18;
        let laid = |id: Id| f.laid()[f.index(id).unwrap()].clone();
        assert_eq!(laid(2).rect, rect(10, 20, 60, 54));
        // Beside the note: stays. Under it: moves down, and hit-testing follows.
        assert_eq!(laid(3).rect.top, 20);
        assert_eq!(laid(4).rect.top, 38 + extra);
        assert_eq!(f.hit(15, 38 + extra + 5).map(|(i, _)| i), f.index(4));
        // The card round it grows; its rows under the note move down, the row above stays.
        assert_eq!(laid(1).rect, rect(0, 0, 200, 100 + extra));
        assert_eq!(laid(1).rows, [20, 60 + extra]);
        // The band under it moves down, and so does the window's height.
        assert_eq!(laid(5).rect, rect(0, 100 + extra, 200, 40));
        assert_eq!(f.height(140), 140 + extra);
        assert_eq!(f.height(0), 140 + extra);
        // Where the text fits, nothing moves.
        let mut f = Form::new(vec![Item::new(
            1,
            Place::new(0, 0, 200, 18),
            Kind::text("aaaa bbbb cccc", Style::Caption, Ink::Muted),
        )]);
        f.lay_out(96, &Fixed(96));
        assert_eq!(f.height(18), 18);
    }

    #[test]
    fn a_card_note_wraps_to_its_column_and_at_line_breaks_and_grows_the_card() {
        assert_eq!(
            wrap(
                "ab cd
ef",
                1000,
                |t| t.len() as i32
            ),
            ["ab cd", "ef"]
        );
        // 200 wide: padding, radio and gap leave 138 px, 19 characters of 7 px.
        let note = "aaaa bbbb cccc dddd
ee"
        .to_owned();
        let one_line = 2 * CHOICE_PAD_Y + 20 + 18;
        let mut f = Form::new(vec![
            Item::new(
                1,
                Place::new(0, 0, 200, one_line),
                Kind::Choice {
                    group: 1,
                    title: "ab".into(),
                    note,
                    selected: true,
                    aside: None,
                },
            ),
            Item::new(2, Place::at(0, one_line), Kind::Switch { on: false }),
        ]);
        f.lay_out(96, &Fixed(96));
        assert_eq!(f.laid()[0].lines, ["aaaa bbbb cccc dddd", "ee"]);
        assert_eq!(f.laid()[0].rect, rect(0, 0, 200, one_line + 18));
        assert_eq!(f.laid()[1].rect.top, one_line + 18, "pushed down");
    }

    #[test]
    fn an_aside_widens_and_heightens_a_card() {
        let aside = Aside {
            w: 150,
            h: 80,
            paint: Rc::new(|_, _, _, _| {}),
        };
        let mut f = Form::new(vec![Item::new(
            1,
            Place::at(0, 0),
            Kind::Choice {
                group: 1,
                title: "ab".into(),
                note: "abcd".into(),
                selected: true,
                aside: Some(aside),
            },
        )]);
        f.lay_out(96, &Fixed(96));
        assert_eq!(
            f.laid()[0].rect,
            rect(0, 0, 16 + 18 + 12 + 28 + 16 + 150 + 16, 14 + 80 + 14)
        );
    }

    #[test]
    fn set_and_set_text_change_values_without_events() {
        let mut f = sample();
        assert!(f.set(SWITCH, Value::On(true)));
        assert!(f.set(FROM, Value::Number(DAY + 30)));
        assert_eq!(f.value(FROM), Some(Value::Number(30)));
        assert!(f.set(CARD_A, Value::On(true)));
        assert_eq!(f.value(CARD_B), Some(Value::On(false)));
        assert!(!f.set(SWITCH, Value::Number(3)));
        assert!(f.set_text(LABEL, "Work week"));
        assert!(!f.set_text(SWITCH, "x"));
        assert_eq!(f.value(LABEL), None);
    }

    // ---- Scrolling ----

    /// A form 1000 tall with its footer band at 920, in a window of at most 600: the content's
    /// part of the window is 520 tall and 400 of the form does not fit.
    fn tall() -> Scroll {
        Scroll::new(1000, Some(920), 600)
    }

    #[test]
    fn a_form_that_fits_does_not_scroll() {
        let mut s = Scroll::new(500, Some(440), 800);
        assert_eq!((s.view(), s.range(), s.content()), (500, 0, 440));
        assert!(!s.scrolls());
        let item = rect(10, 300, 80, 32);
        let save = rect(10, 456, 80, 32);
        assert_eq!(s.on_client(item), item);
        assert_eq!(s.on_client(save), save);
        assert_eq!(s.in_form(10, 470), (10, 470));
        assert!(!s.scroll_by(50));
        assert!(!s.wheel(-120, 3, 20));
        assert!(!s.reveal(item, 16));
        assert_eq!(s.thumb(600, 96, 0.0), None);
        assert_eq!(s.bar(600, 96), None);
        // Exactly the room it needs: still no scroll.
        assert!(!Scroll::new(500, Some(440), 500).scrolls());
    }

    #[test]
    fn a_form_too_tall_scrolls_its_content_under_a_footer_that_stays() {
        let mut s = tall();
        assert_eq!((s.view(), s.range(), s.content()), (600, 400, 520));
        let item = rect(24, 100, 80, 32);
        let save = rect(400, 936, 88, 32);
        // At the top the footer band sits at the bottom of the window.
        assert_eq!(s.on_client(item), item);
        assert_eq!(s.on_client(save), rect(400, 536, 88, 32));
        assert!(s.shown(item) && s.shown(save));
        assert!(!s.shown(rect(24, 700, 80, 32)), "under the band");
        // Scrolled to the end, the content moves up and the band stays.
        assert!(s.scroll_to(10_000));
        assert_eq!(s.offset(), 400);
        assert_eq!(s.on_client(item), rect(24, -300, 80, 32));
        assert_eq!(s.on_client(save), rect(400, 536, 88, 32));
        assert!(!s.shown(item) && s.shown(rect(24, 900, 80, 16)));
        // Client points map back: the band at the bottom, the content above it.
        assert_eq!(s.in_form(400, 540), (400, 940));
        assert_eq!(s.in_form(24, 519), (24, 919));
        assert!(s.scroll_to(-5));
        assert_eq!(s.offset(), 0);
        assert_eq!(s.in_form(24, 519), (24, 519));
    }

    #[test]
    fn the_visible_part_is_cut_at_the_client_top_and_at_the_band() {
        let mut s = tall();
        let save = rect(400, 936, 88, 32);
        assert_eq!(
            s.visible(rect(24, 100, 80, 32)),
            Some(rect(24, 100, 80, 32))
        );
        assert_eq!(
            s.visible(rect(24, 500, 80, 32)),
            Some(rect(24, 500, 80, 20)),
            "into the band"
        );
        assert_eq!(s.visible(rect(24, 700, 80, 32)), None, "under the band");
        assert_eq!(s.visible(save), Some(rect(400, 536, 88, 32)));
        s.scroll_to(110);
        assert_eq!(
            s.visible(rect(24, 100, 80, 32)),
            Some(rect(24, 0, 80, 22)),
            "past the top"
        );
        assert_eq!(s.visible(rect(24, 60, 80, 32)), None);
        assert_eq!(
            s.visible(save),
            Some(rect(400, 536, 88, 32)),
            "the band stays"
        );
    }

    #[test]
    fn a_window_keeps_room_for_the_footer_and_a_form_with_no_band_scrolls_whole() {
        let s = Scroll::new(1000, Some(920), 50);
        assert_eq!((s.view(), s.content(), s.range()), (80, 0, 920));
        let s = Scroll::new(1000, None, 600);
        assert_eq!((s.view(), s.content(), s.range()), (600, 600, 400));
        assert_eq!(s.on_client(rect(0, 990, 10, 10)), rect(0, 990, 10, 10));
    }

    #[test]
    fn focus_scrolls_a_control_into_view_with_room_round_it() {
        let mut s = tall();
        // Below the window: it comes up until its bottom and the room show.
        assert!(s.reveal(rect(24, 700, 80, 32), 16));
        assert_eq!(s.offset(), 732 + 16 - 520);
        let r = s.on_client(rect(24, 700, 80, 32));
        assert_eq!(r.bottom + 16, s.content());
        // Already in view: nothing moves.
        assert!(!s.reveal(rect(24, 600, 80, 32), 16));
        // Above the window: it comes down until its top and the room show.
        assert!(s.reveal(rect(24, 100, 80, 32), 16));
        assert_eq!(s.offset(), 84);
        // The last control: as far as the range goes, no further.
        s.reveal(rect(24, 880, 80, 32), 16);
        assert_eq!(s.offset(), 400);
        // A control in the footer band is always in view.
        assert!(!s.reveal(rect(400, 936, 88, 32), 16));
        assert_eq!(s.offset(), 400);
        // Taller than the window's content part: its top shows.
        assert!(s.reveal(rect(24, 200, 80, 600), 16));
        assert_eq!(s.offset(), 184);
    }

    #[test]
    fn the_wheel_scrolls_lines_a_notch_and_adds_up_touchpad_parts() {
        let mut s = tall();
        // A notch toward the user: three lines of 20 down.
        assert!(s.wheel(-120, 3, 20));
        assert_eq!(s.offset(), 60);
        assert!(s.wheel(120, 3, 20));
        assert_eq!(s.offset(), 0);
        // Away from the user at the top: nothing, and nothing saved for later.
        assert!(!s.wheel(120, 3, 20));
        assert!(s.wheel(-120, 3, 20));
        assert_eq!(s.offset(), 60);
        // A touchpad sends parts of a notch: two of 1/120 at 60 a notch make a pixel.
        let mut s = tall();
        assert!(!s.wheel(-1, 3, 20));
        assert!(s.wheel(-1, 3, 20));
        assert_eq!(s.offset(), 1);
        for _ in 0..4 {
            s.wheel(-30, 3, 20);
        }
        assert_eq!(s.offset(), 61);
        // A page a notch, as the user may set it, and no scroll at 0 lines.
        let mut s = tall();
        assert!(s.wheel(-120, u32::MAX, 20));
        assert_eq!(s.offset(), 400, "a page of 520, held in the range");
        assert!(!s.wheel(120, 0, 20));
    }

    #[test]
    fn the_thumb_runs_down_its_track_as_the_content_scrolls() {
        let mut s = tall();
        // The track is the content's part, 2 in from its ends: 516 long. The thumb is as long
        // against it as 520 shown is against 920.
        let thumb = s.thumb(600, 96, 0.0).unwrap();
        assert_eq!(
            (thumb.x, thumb.y, thumb.w, thumb.h),
            (594.0, 2.0, 4.0, 291.0)
        );
        let hot = s.thumb(600, 96, 1.0).unwrap();
        assert_eq!((hot.x, hot.w), (590.0, 8.0));
        assert_eq!(s.bar(600, 96), Some(rect(588, 0, 12, 520)));
        s.scroll_to(400);
        let end = s.thumb(600, 96, 0.0).unwrap();
        assert_eq!(end.y + end.h, 518.0);
        // At 200 %, every size doubles.
        let big = Scroll::new(2000, Some(1840), 1200);
        let b = big.thumb(1200, 192, 0.0).unwrap();
        assert_eq!((b.x, b.y, b.w), (1188.0, 4.0, 8.0));
        // A long form keeps the thumb at least 32.
        let long = Scroll::new(20_000, Some(19_920), 600);
        assert_eq!(long.thumb(600, 96, 0.0).unwrap().h, 32.0);
    }

    #[test]
    fn dragging_the_thumb_and_clicking_the_track_scroll() {
        let mut s = tall();
        assert!(s.on_thumb(100, 96));
        assert!(!s.on_thumb(400, 96));
        // The thumb's free run is 516 - 291 = 225 for a range of 400.
        assert!(s.drag(0, 225, 96));
        assert_eq!(s.offset(), 400);
        assert!(s.drag(400, -90, 96));
        assert_eq!(s.offset(), 240);
        // A click under the thumb pages down, above it pages up.
        let mut s = tall();
        assert!(s.page_toward(500, 96));
        assert_eq!(s.offset(), 400);
        assert!(s.page_toward(10, 96));
        assert_eq!(s.offset(), 0);
    }

    #[test]
    fn the_footer_is_the_last_band() {
        let band = |y| Item::new(0, Place::new(0, y, 300, 64), Kind::Band);
        let mut f = Form::new(vec![
            band(0),
            Item::new(
                SAVE,
                Place::at(10, 216),
                Kind::button("Save", ButtonKind::Primary),
            ),
            band(200),
        ]);
        f.lay_out(144, &Fixed(144));
        assert_eq!(f.footer(), Some(300));
        assert_eq!(f.height(0), 396);
        let s = Scroll::new(f.height(0), f.footer(), 200);
        assert_eq!(s.on_client(f.laid()[1].rect).top, 324 - 196);
        assert_eq!(sample().footer(), None);
    }
}
