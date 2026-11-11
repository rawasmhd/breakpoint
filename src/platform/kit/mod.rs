//! The form kit: one painted window type and the controls the dialogs need (issue #77).
//!
//! A kit window is a normal top-level window with the system caption and frame. Windows draws
//! the frame, tinted to the theme through DWM; breakpoint paints the whole client area with GDI+
//! and GDI text, double-buffered, in the tokens of [`super::theme`]. There are no child windows:
//! every control is painted, hit-tested and driven from the keyboard here.
//!
//! The caller describes the form as data, a list of [`Item`]s with places at 96 DPI, and gets
//! [`Event`]s back through a callback ([`window::open`]). The kit scales the places with [`theme::px`] for the
//! window's DPI and lays out again when the DPI changes.
//!
//! - [`form`] is the state machine in plain Rust: layout, hit-testing, focus order, keys, stepper
//!   math, the drop-down menu and the hover and press timing. Unit tests drive it without a window.
//! - [`draw`] paints a laid-out form and its menu on a device context.
//! - [`window`] is the Win32 side: the window, its menu window, DPI, theme and high contrast.
//!
//! [`theme::px`]: super::theme::px

pub mod draw;
pub mod form;
#[cfg(test)]
pub(crate) mod shots;
pub mod window;

use std::fmt;
use std::rc::Rc;

use super::paint::{Box2, Canvas, Glyph};
use super::theme::{Colours, Style};

/// The caller's name for an item, given back in [`Event`]s.
pub type Id = u16;

/// Where an item goes, in pixels at 96 DPI from the client area's top-left corner.
///
/// A width or height of 0 asks for the control's own size: the sizes in the issue's controls
/// table, or the text's measured width and lines.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Place {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Place {
    /// At `x`, `y`, in the item's own size.
    pub const fn at(x: i32, y: i32) -> Place {
        Place { x, y, w: 0, h: 0 }
    }

    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Place {
        Place { x, y, w, h }
    }
}

/// Which text colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ink {
    /// `ink`.
    Main,
    /// `ink-muted`.
    Muted,
}

/// The button kinds of DESIGN.md §6.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonKind {
    Primary,
    Secondary,
    Ghost,
}

/// One drop-down menu item: the name, and an optional `caption` line under it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub note: Option<String>,
}

impl Entry {
    pub fn new(name: &str, note: Option<&str>) -> Entry {
        Entry {
            name: name.to_owned(),
            note: note.map(str::to_owned),
        }
    }
}

/// What a selectable card's paint hook draws: the canvas, the box at the card's right (scaled),
/// the colours in force and the DPI.
pub type PaintHook = dyn Fn(&Canvas, Box2, &Colours, u32);

/// Content on the right of a selectable card, such as the wizard's day timeline: `w` by `h` at
/// 96 DPI, centred on the card's height, `paint` called on every paint with the scaled box.
#[derive(Clone)]
pub struct Aside {
    pub w: i32,
    pub h: i32,
    pub paint: Rc<PaintHook>,
}

impl fmt::Debug for Aside {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Aside({} x {})", self.w, self.h)
    }
}

impl PartialEq for Aside {
    fn eq(&self, other: &Aside) -> bool {
        self.w == other.w && self.h == other.h && Rc::ptr_eq(&self.paint, &other.paint)
    }
}

/// The controls and shapes a form is made of.
#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    /// A `surface-raised` card with `radius-md` corners. `rows` are the hairlines between its
    /// rows, as y offsets from the card's top at 96 DPI; they run inside the `space-4` padding.
    Card {
        rows: Vec<i32>,
    },
    /// The footer band: `surface-sunken` with a `line` hairline on its top edge.
    Band,
    /// Text in one type style. With a width in its place it wraps at spaces.
    Text {
        text: String,
        style: Style,
        ink: Ink,
    },
    /// A nudge glyph at `size-glyph`, in its nudge colour.
    Glyph(Glyph),
    /// A push button. Enter presses the `default` one; Esc and the caption × press the `cancel`
    /// one. One button may be both.
    Button {
        label: String,
        kind: ButtonKind,
        default: bool,
        cancel: bool,
    },
    Switch {
        on: bool,
    },
    /// A checkbox, with an optional `body` label right of it that also takes clicks.
    Checkbox {
        on: bool,
        label: String,
    },
    /// A day pill, such as one work day.
    Day {
        label: String,
        on: bool,
    },
    /// A number stepper in `min..=max` with `unit` after the number ("min"). A typed value
    /// outside the range stays as typed, so the caller's Save can name it (SPEC §9.2).
    Stepper {
        value: i32,
        min: i32,
        max: i32,
        step: i32,
        unit: String,
    },
    /// A time stepper: `minutes` after midnight shown as `HH:mm`, `step` minutes per step,
    /// wrapping round midnight. Any time can be typed.
    Time {
        minutes: i32,
        step: i32,
    },
    /// A segmented toggle; one Tab stop, arrows move the selection.
    Segmented {
        options: Vec<String>,
        selected: usize,
    },
    /// A drop-down: the field shows `entries[current]`, or `other` when `current` is `None`
    /// (a value the menu does not offer, such as Custom).
    DropDown {
        entries: Vec<Entry>,
        current: Option<usize>,
        other: String,
    },
    /// A selectable card. Cards with the same `group` act as one radio group: one Tab stop,
    /// arrows move the selection.
    Choice {
        group: u16,
        title: String,
        note: String,
        selected: bool,
        aside: Option<Aside>,
    },
}

impl Kind {
    pub fn text(text: &str, style: Style, ink: Ink) -> Kind {
        Kind::Text {
            text: text.to_owned(),
            style,
            ink,
        }
    }

    pub fn button(label: &str, kind: ButtonKind) -> Kind {
        Kind::Button {
            label: label.to_owned(),
            kind,
            default: false,
            cancel: false,
        }
    }

    /// Whether Tab can stop here or the item can take focus some other way.
    pub fn focusable(&self) -> bool {
        !matches!(
            self,
            Kind::Card { .. } | Kind::Band | Kind::Text { .. } | Kind::Glyph(_)
        )
    }
}

/// One item of a form.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub id: Id,
    pub at: Place,
    pub kind: Kind,
}

impl Item {
    pub fn new(id: Id, at: Place, kind: Kind) -> Item {
        Item { id, at, kind }
    }
}

/// A control's value, as [`Event::Changed`] reports it and [`window::Handle::set`] takes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Value {
    /// Switch, checkbox, day pill; for a selectable card, whether it is the selected one.
    On(bool),
    /// Stepper; time stepper in minutes after midnight.
    Number(i32),
    /// Segmented toggle (always `Some`); drop-down (`None` shows its `other` text).
    Index(Option<usize>),
}

/// What the user did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// A button was pressed: clicked, Space, Enter for the default, Esc or × for the cancel.
    Pressed(Id),
    /// A control's value changed by the user. A selectable card reports `On(true)` when it
    /// becomes the selected one.
    Changed(Id, Value),
    /// Esc or × with no cancel button. The window stays until the caller closes it.
    Close,
}
