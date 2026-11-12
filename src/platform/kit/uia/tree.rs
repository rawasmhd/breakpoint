//! The accessible tree of a form, in plain Rust: which elements a screen reader sees, their
//! names, roles and states, what an automation client may do to them, and what changed
//! between two moments. [`super`] turns it into UI Automation providers.
//!
//! The tree is read off the laid-out form, so no caller has to describe it twice:
//!
//! - The window is the root. A card is a group, named by the heading right above it or else by
//!   the first text inside it. In a card with hairlines, a row with two or more controls is a
//!   group of its own, named by its first text: "Work days", "Stand up and move".
//! - Selectable cards of one radio group sit in a group named by the heading above them.
//!   A segmented toggle is a group of radio buttons, one per segment. A drop-down holds a list
//!   of its menu entries, on screen while the menu is open.
//! - A control's name is its own label (button, checkbox, day pill, selectable card), else the
//!   nearest text left of it on its line with no other control between, else its row's or
//!   card's name; a segmented toggle with no such text takes its segments' words, "Soft /
//!   Firm". A stepper's name ends with its unit: "Every (minutes)".
//! - A note (text in `ink-muted`) describes the control it sits to the right of, the control
//!   whose label it sits under, the control right above it, or the one control of its card or
//!   row, in that order. It stays a text element too.
//!
//! - A line of mixed styles (the flyout's countdown, its Today count) is one text element with
//!   its pieces' words, and counts as a note when its first piece is `ink-muted`.
//!
//! Glyphs, the status dot, hairlines and the footer band are decoration and are left out.

use windows::Win32::Foundation::RECT;

use super::super::form::{DAY, Form, Look, Part, parse_number, parse_time, time_text};
use super::super::{Event, Id, Ink, Item, Kind, Value};
use crate::platform::theme::{PAGE_STEPS, Style, px};

/// One element of the tree. Item indexes are positions in the form's item list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    /// The window.
    Root,
    /// An item: a card, a text or a control.
    Item(usize),
    /// Row `.1` of the card at item `.0`, when that row is a group of its own.
    Row(usize, usize),
    /// The selectable cards of one radio group.
    Choices(u16),
    /// Segment `.1` of the segmented toggle at item `.0`.
    Option(usize, usize),
    /// The menu list of the drop-down at item `.0`.
    Menu(usize),
    /// Entry `.1` of that menu.
    Entry(usize, usize),
}

impl Key {
    /// The numbers that tell this element from its siblings, for its runtime id. Empty for the
    /// root, whose runtime id comes from its window.
    pub fn runtime_id(self) -> Vec<i32> {
        let n = |v: usize| v as i32;
        match self {
            Key::Root => Vec::new(),
            Key::Item(i) => vec![1, n(i)],
            Key::Row(c, r) => vec![2, n(c), n(r)],
            Key::Choices(g) => vec![3, g.into()],
            Key::Option(i, k) => vec![4, n(i), n(k)],
            Key::Menu(i) => vec![5, n(i)],
            Key::Entry(i, k) => vec![6, n(i), n(k)],
        }
    }

    /// The form item this element belongs to.
    pub fn item(self) -> Option<usize> {
        match self {
            Key::Item(i) | Key::Option(i, _) | Key::Menu(i) | Key::Entry(i, _) => Some(i),
            Key::Root | Key::Row(..) | Key::Choices(_) => None,
        }
    }
}

/// What an element is to a screen reader. [`super`] maps each to a UI Automation control type
/// and its patterns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// The window; its frame and caption come from Windows.
    Root,
    /// A card, a row of a card, or the selectable cards of one radio group (with selection).
    Group,
    Text,
    /// Invoke.
    Button,
    /// Toggle. Checkboxes and day pills.
    CheckBox,
    /// Toggle; a check box that reads as a toggle switch.
    Switch,
    /// A segmented toggle or a radio group of selectable cards: Selection.
    RadioGroup,
    /// A segment or a selectable card: SelectionItem.
    Radio,
    /// A stepper or time stepper: RangeValue and Value.
    Spinner,
    /// A drop-down: ExpandCollapse, Selection and a read-only Value.
    ComboBox,
    /// A drop-down's menu: Selection.
    List,
    /// A menu entry: SelectionItem.
    ListItem,
}

impl Role {
    /// Whether the element can take the keyboard focus.
    pub fn focusable(self) -> bool {
        matches!(
            self,
            Role::Button
                | Role::CheckBox
                | Role::Switch
                | Role::Radio
                | Role::Spinner
                | Role::ComboBox
                | Role::ListItem
        )
    }

    fn word(self) -> &'static str {
        match self {
            Role::Root => "window",
            Role::Group => "group",
            Role::Text => "text",
            Role::Button => "button",
            Role::CheckBox => "checkbox",
            Role::Switch => "switch",
            Role::RadioGroup => "radiogroup",
            Role::Radio => "radio",
            Role::Spinner => "spinner",
            Role::ComboBox => "combobox",
            Role::List => "list",
            Role::ListItem => "listitem",
        }
    }
}

/// Where an element is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Area {
    /// Nowhere on screen: a closed menu, or the root, whose place Windows knows.
    None,
    /// In the form's pixels, as [`Form::laid`] has them; the window maps them to the screen.
    Form(RECT),
    /// On screen already: the menu window's entries.
    Screen(RECT),
}

/// A stepper's numbers for the RangeValue pattern.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Range {
    pub value: f64,
    pub min: f64,
    pub max: f64,
    pub small: f64,
    pub large: f64,
}

/// One element and everything a screen reader asks about it.
#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub key: Key,
    pub parent: Option<Key>,
    pub children: Vec<Key>,
    pub role: Role,
    pub name: String,
    /// The kit id, as the automation id; `None` for decoration and containers.
    pub id: Option<Id>,
    pub area: Area,
    /// The text whose words make the name, when it is not the element's own.
    pub labeled_by: Option<Key>,
    /// The note that describes the element.
    pub described_by: Option<Key>,
    /// The words of that note, or a selectable card's or menu entry's own note line.
    pub help: String,
    /// 1 for the title, 2 for a text that names a group, else 0.
    pub heading: u8,
    pub toggle: Option<bool>,
    pub selected: Option<bool>,
    pub range: Option<Range>,
    pub value: Option<String>,
    pub expanded: Option<bool>,
    /// A time stepper: its value is a time of day, typed as `8:30`.
    pub time: bool,
    /// Place in its set, from 1, and the set's size: radio buttons and menu entries.
    pub set: Option<(usize, usize)>,
}

impl Node {
    fn new(key: Key, role: Role, name: String, area: Area) -> Node {
        Node {
            key,
            parent: None,
            children: Vec::new(),
            role,
            name,
            id: None,
            area,
            labeled_by: None,
            described_by: None,
            help: String::new(),
            heading: 0,
            toggle: None,
            selected: None,
            range: None,
            value: None,
            expanded: None,
            time: false,
            set: None,
        }
    }
}

/// Which way to go from an element.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Nav {
    Parent,
    Next,
    Previous,
    First,
    Last,
}

/// The accessible tree of a form at one moment.
#[derive(Clone, Debug, PartialEq)]
pub struct Tree {
    nodes: Vec<Node>,
    focus: Option<Key>,
}

impl Tree {
    /// The tree of `form`, which must be laid out. `menu` gives the open menu's entries on
    /// screen, in order.
    pub fn new(form: &Form, menu: Option<&[RECT]>) -> Tree {
        Builder::new(form, menu).build()
    }

    pub fn nodes(&self) -> &[Node] {
        &self.nodes
    }

    pub fn get(&self, key: Key) -> Option<&Node> {
        self.nodes.iter().find(|n| n.key == key)
    }

    /// The element that has the focus inside the form: the focused control, the selected
    /// segment of a focused segmented toggle, or the highlighted entry of an open menu.
    pub fn focus(&self) -> Option<Key> {
        self.focus
    }

    pub fn navigate(&self, key: Key, nav: Nav) -> Option<Key> {
        let node = self.get(key)?;
        let siblings = || {
            let parent = self.get(node.parent?)?;
            let at = parent.children.iter().position(|&k| k == key)?;
            Some((&parent.children, at))
        };
        match nav {
            Nav::Parent => node.parent,
            Nav::First => node.children.first().copied(),
            Nav::Last => node.children.last().copied(),
            Nav::Next => {
                let (all, at) = siblings()?;
                all.get(at + 1).copied()
            }
            Nav::Previous => {
                let (all, at) = siblings()?;
                at.checked_sub(1).map(|p| all[p])
            }
        }
    }

    /// How deep `key` is: 0 for the root.
    pub fn depth(&self, key: Key) -> usize {
        let mut depth = 0;
        let mut at = self.get(key).and_then(|n| n.parent);
        while let Some(k) = at {
            depth += 1;
            at = self.get(k).and_then(|n| n.parent);
        }
        depth
    }

    /// The selected elements of a selection container: a radio group's selected radio, a
    /// drop-down's or menu's current entry.
    pub fn selection(&self, key: Key) -> Vec<Key> {
        let Some(node) = self.get(key) else {
            return Vec::new();
        };
        let list = match node.role {
            Role::ComboBox => match self.get(Key::Menu(key.item().unwrap_or_default())) {
                Some(menu) => menu,
                None => return Vec::new(),
            },
            _ => node,
        };
        list.children
            .iter()
            .copied()
            .filter(|&k| self.get(k).is_some_and(|n| n.selected == Some(true)))
            .collect()
    }

    /// The tree as indented lines, one element each: role, name, state and note. The tests
    /// read it to check the whole tree at once.
    pub fn outline(&self) -> String {
        let mut out = String::new();
        self.outline_from(Key::Root, 0, &mut out);
        out
    }

    fn outline_from(&self, key: Key, depth: usize, out: &mut String) {
        let Some(n) = self.get(key) else {
            return;
        };
        let mut line = format!("{}{}", "  ".repeat(depth), n.role.word());
        if !n.name.is_empty() {
            line += &format!(" {:?}", n.name);
        }
        if n.heading > 0 {
            line += &format!(" h{}", n.heading);
        }
        if let Some(on) = n.toggle {
            line += if on { " on" } else { " off" };
        }
        if n.selected == Some(true) {
            line += " selected";
        }
        if let Some((at, of)) = n.set {
            line += &format!(" {at}/{of}");
        }
        if let Some(v) = &n.value {
            line += &format!(" ={v:?}");
        }
        if let Some(r) = n.range {
            line += &format!(" [{}..={} by {}]", r.min, r.max, r.small);
        }
        if let Some(open) = n.expanded {
            line += if open { " open" } else { " closed" };
        }
        if self.focus == Some(key) {
            line += " focused";
        }
        if !n.help.is_empty() {
            line += &format!(" -- {:?}", n.help);
        }
        out.push_str(&line);
        out.push('\n');
        for &child in &n.children {
            self.outline_from(child, depth + 1, out);
        }
    }
}

// ---- Building ----

fn mid(r: &RECT) -> (i32, i32) {
    ((r.left + r.right) / 2, (r.top + r.bottom) / 2)
}

fn contains(r: &RECT, (x, y): (i32, i32)) -> bool {
    x >= r.left && x < r.right && y >= r.top && y < r.bottom
}

/// Whether two boxes share a line: one's middle lies within the other's height.
fn same_line(a: &RECT, b: &RECT) -> bool {
    let (ay, by) = (mid(a).1, mid(b).1);
    (ay >= b.top && ay < b.bottom) || (by >= a.top && by < a.bottom)
}

fn union(rects: impl IntoIterator<Item = RECT>) -> Option<RECT> {
    rects.into_iter().reduce(|a, b| RECT {
        left: a.left.min(b.left),
        top: a.top.min(b.top),
        right: a.right.max(b.right),
        bottom: a.bottom.max(b.bottom),
    })
}

/// A unit as words: "min" reads "minutes".
fn unit_words(unit: &str) -> &str {
    match unit {
        "min" => "minutes",
        "h" => "hours",
        "s" => "seconds",
        _ => unit,
    }
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

struct Builder<'a> {
    form: &'a Form,
    items: &'a [Item],
    menu: Option<&'a [RECT]>,
    looks: Vec<Look>,
    /// The card each item lies in.
    card_of: Vec<Option<usize>>,
    /// The text that labels each control without a label of its own.
    label: Vec<Option<usize>>,
    /// The control each note describes.
    note_for: Vec<Option<usize>>,
    /// Slack for "right above" and "right under", in pixels.
    slack: i32,
    nodes: Vec<Node>,
}

impl<'a> Builder<'a> {
    fn new(form: &'a Form, menu: Option<&'a [RECT]>) -> Builder<'a> {
        let items = form.items();
        let mut b = Builder {
            form,
            items,
            menu,
            looks: form.looks(),
            card_of: Vec::new(),
            label: vec![None; items.len()],
            note_for: vec![None; items.len()],
            slack: px(4, form.dpi()),
            nodes: Vec::new(),
        };
        b.card_of = (0..items.len()).map(|i| b.find_card(i)).collect();
        for i in 0..items.len() {
            if b.items[i].kind.focusable() && b.own_label(i).is_none() {
                b.label[i] = b.line_label(i);
            }
        }
        for t in 0..items.len() {
            if b.is_text(t, Ink::Muted) {
                b.note_for[t] = b.described(t);
            }
        }
        b
    }

    fn rect(&self, i: usize) -> RECT {
        self.form.laid()[i].rect
    }

    fn is_text(&self, i: usize, which: Ink) -> bool {
        match &self.items[i].kind {
            Kind::Text { ink, .. } => *ink == which,
            Kind::Spans { spans, .. } => spans.first().is_some_and(|s| s.ink == which),
            _ => false,
        }
    }

    fn text(&self, i: usize) -> String {
        match &self.items[i].kind {
            Kind::Text { text, .. } => one_line(text),
            Kind::Spans { spans, .. } => {
                one_line(&spans.iter().map(|s| s.text.as_str()).collect::<String>())
            }
            _ => String::new(),
        }
    }

    fn is_control(&self, i: usize) -> bool {
        self.items[i].kind.focusable()
    }

    fn find_card(&self, i: usize) -> Option<usize> {
        if matches!(self.items[i].kind, Kind::Card { .. } | Kind::Band) {
            return None;
        }
        let at = mid(&self.rect(i));
        (0..i).rev().find(|&c| {
            matches!(self.items[c].kind, Kind::Card { .. }) && contains(&self.rect(c), at)
        })
    }

    /// The card and row item `i` lies in.
    fn row_of(&self, i: usize) -> Option<(usize, usize)> {
        let c = self.card_of[i]?;
        let y = mid(&self.rect(i)).1;
        Some((
            c,
            self.form.laid()[c].rows.iter().filter(|&&r| r <= y).count(),
        ))
    }

    /// How many controls a card row holds; a radio group of selectable cards counts once.
    fn controls_in(&self, c: usize, r: usize) -> usize {
        let mut groups = Vec::new();
        (0..self.items.len())
            .filter(|&i| self.is_control(i) && self.row_of(i) == Some((c, r)))
            .filter(|&i| match self.items[i].kind {
                Kind::Choice { group, .. } if groups.contains(&group) => false,
                Kind::Choice { group, .. } => {
                    groups.push(group);
                    true
                }
                _ => true,
            })
            .count()
    }

    /// Whether row `r` of card `c` is a group of its own: the card has rows, and this one
    /// holds two controls or more.
    fn row_group(&self, c: usize, r: usize) -> bool {
        !self.form.laid()[c].rows.is_empty() && self.controls_in(c, r) >= 2
    }

    /// The group an item's element sits in, before selectable cards get their own.
    fn container(&self, i: usize) -> Key {
        match self.row_of(i) {
            Some((c, r)) if self.row_group(c, r) => Key::Row(c, r),
            Some((c, _)) => Key::Item(c),
            None => Key::Root,
        }
    }

    fn members(&self, group: u16) -> Vec<usize> {
        (0..self.items.len())
            .filter(|&i| matches!(self.items[i].kind, Kind::Choice { group: g, .. } if g == group))
            .collect()
    }

    fn parent(&self, key: Key) -> Option<Key> {
        Some(match key {
            Key::Root => return None,
            Key::Item(i) => match self.items[i].kind {
                Kind::Choice { group, .. } => Key::Choices(group),
                _ => self.container(i),
            },
            Key::Row(c, _) => Key::Item(c),
            Key::Choices(g) => self.container(*self.members(g).first()?),
            Key::Option(i, _) | Key::Menu(i) => Key::Item(i),
            Key::Entry(i, _) => Key::Menu(i),
        })
    }

    // ---- Names ----

    fn own_label(&self, i: usize) -> Option<String> {
        match &self.items[i].kind {
            Kind::Button { label, .. } | Kind::Day { label, .. } => Some(label.clone()),
            Kind::Checkbox { label, .. } if !label.is_empty() => Some(label.clone()),
            Kind::Choice { title, .. } => Some(title.clone()),
            _ => None,
        }
    }

    /// The nearest `ink` text left of control `i` on its line, in its card, with no other
    /// control between.
    fn line_label(&self, i: usize) -> Option<usize> {
        let r = self.rect(i);
        let blocked = |t: &RECT| {
            (0..self.items.len()).any(|b| {
                let br = self.rect(b);
                b != i
                    && self.is_control(b)
                    && self.card_of[b] == self.card_of[i]
                    && same_line(&br, &r)
                    && br.left >= t.right
                    && br.right <= r.left
            })
        };
        (0..self.items.len())
            .filter(|&t| self.is_text(t, Ink::Main) && self.card_of[t] == self.card_of[i])
            .filter(|&t| {
                let tr = self.rect(t);
                same_line(&tr, &r) && tr.right <= r.left + self.slack && !blocked(&tr)
            })
            .max_by_key(|&t| self.rect(t).right)
    }

    /// The heading right above item `f`: the `ink` text before it in the list, past any notes,
    /// when it sits above it.
    fn heading_before(&self, f: usize) -> Option<usize> {
        let top = self.rect(f).top;
        (0..f)
            .rev()
            .find(|&j| !self.is_text(j, Ink::Muted))
            .filter(|&j| self.is_text(j, Ink::Main) && self.rect(j).bottom <= top + self.slack)
    }

    fn first_text(&self, within: impl Fn(usize) -> bool) -> Option<usize> {
        (0..self.items.len()).find(|&t| self.is_text(t, Ink::Main) && within(t))
    }

    /// The text that names a card.
    fn card_name(&self, c: usize) -> Option<usize> {
        self.heading_before(c)
            .or_else(|| self.first_text(|t| self.card_of[t] == Some(c)))
    }

    fn row_name(&self, c: usize, r: usize) -> Option<usize> {
        self.first_text(|t| self.row_of(t) == Some((c, r)))
    }

    /// The text a control without a label falls back on: its row's name, else its card's.
    fn fallback(&self, i: usize) -> Option<usize> {
        let (c, r) = self.row_of(i)?;
        let row = if self.row_group(c, r) {
            self.row_name(c, r)
        } else {
            None
        };
        row.or_else(|| self.card_name(c))
    }

    /// The control note `t` describes, by the rules in the module docs.
    fn described(&self, t: usize) -> Option<usize> {
        let tr = self.rect(t);
        let card = self.card_of[t];
        let controls = || {
            (0..self.items.len()).filter(move |&b| self.is_control(b) && self.card_of[b] == card)
        };
        // Right of a control on its line.
        let beside = controls()
            .filter(|&b| {
                let br = self.rect(b);
                same_line(&br, &tr) && br.right <= tr.left + self.slack
            })
            .max_by_key(|&b| self.rect(b).right);
        let near = |a: i32, b: i32| (a - b).abs() <= self.slack;
        // Under the label of a control.
        let under_label = || {
            (0..self.items.len()).find_map(|l| {
                let lr = self.rect(l);
                let labels = (0..self.items.len()).find(|&c| self.label[c] == Some(l));
                (self.is_text(l, Ink::Main) && near(lr.bottom, tr.top) && near(lr.left, tr.left))
                    .then_some(labels)
                    .flatten()
            })
        };
        // Right under a control.
        let under_control = || {
            controls().find(|&b| {
                let br = self.rect(b);
                near(br.bottom, tr.top) && br.left < tr.right && tr.left < br.right
            })
        };
        // The one control of its card or row.
        let only = || {
            let (c, r) = self.row_of(t)?;
            let mut in_place = controls().filter(|&b| {
                if self.form.laid()[c].rows.is_empty() {
                    true
                } else {
                    self.row_of(b) == Some((c, r))
                }
            });
            let first = in_place.next()?;
            in_place.next().is_none().then_some(first)
        };
        beside
            .or_else(under_label)
            .or_else(under_control)
            .or_else(only)
    }

    // ---- Nodes ----

    fn build(mut self) -> Tree {
        self.nodes
            .push(Node::new(Key::Root, Role::Root, String::new(), Area::None));
        for i in 0..self.items.len() {
            if matches!(
                self.items[i].kind,
                Kind::Glyph(_) | Kind::Band | Kind::Rule | Kind::Dot { .. }
            ) {
                continue;
            }
            self.ensure(Key::Item(i));
            match &self.items[i].kind {
                Kind::Segmented { options, .. } => {
                    for k in 0..options.len() {
                        self.ensure(Key::Option(i, k));
                    }
                }
                Kind::DropDown { entries, .. } => {
                    self.ensure(Key::Menu(i));
                    for k in 0..entries.len() {
                        self.ensure(Key::Entry(i, k));
                    }
                }
                _ => {}
            }
        }
        let focus = self.focus();
        Tree {
            nodes: self.nodes,
            focus,
        }
    }

    fn focus(&self) -> Option<Key> {
        let i = self.form.index(self.form.focused()?)?;
        Some(match &self.items[i].kind {
            Kind::DropDown { .. } => match self.form.menu() {
                Some(menu) if menu.item == i => menu.hot.map_or(Key::Item(i), |k| Key::Entry(i, k)),
                _ => Key::Item(i),
            },
            Kind::Segmented { selected, .. } => Key::Option(i, *selected),
            _ => Key::Item(i),
        })
    }

    /// Adds the element `key` under its parent, adding the parent first if need be, so the
    /// children keep the list's reading order.
    fn ensure(&mut self, key: Key) {
        if self.nodes.iter().any(|n| n.key == key) {
            return;
        }
        let parent = self.parent(key).unwrap_or(Key::Root);
        self.ensure(parent);
        let mut node = self.node(key);
        node.parent = Some(parent);
        if let Some(p) = self.nodes.iter_mut().find(|n| n.key == parent) {
            p.children.push(key);
        }
        self.nodes.push(node);
    }

    fn named_by(&self, node: &mut Node, text: Option<usize>) {
        if let Some(t) = text {
            node.name = self.text(t);
            node.labeled_by = Some(Key::Item(t));
        }
    }

    fn node(&self, key: Key) -> Node {
        let laid = self.form.laid();
        match key {
            Key::Root => Node::new(key, Role::Root, String::new(), Area::None),
            Key::Item(i) => self.item_node(i),
            Key::Row(c, r) => {
                let card = laid[c].rect;
                let lines = &laid[c].rows;
                let top = if r == 0 { card.top } else { lines[r - 1] };
                let bottom = lines.get(r).copied().unwrap_or(card.bottom);
                let area = RECT {
                    top,
                    bottom,
                    ..card
                };
                let mut node = Node::new(key, Role::Group, String::new(), Area::Form(area));
                self.named_by(&mut node, self.row_name(c, r));
                node
            }
            Key::Choices(g) => {
                let members = self.members(g);
                let area = union(members.iter().map(|&i| self.rect(i)));
                let mut node = Node::new(
                    key,
                    Role::RadioGroup,
                    String::new(),
                    area.map_or(Area::None, Area::Form),
                );
                self.named_by(
                    &mut node,
                    members.first().and_then(|&f| self.heading_before(f)),
                );
                node
            }
            Key::Option(i, k) => {
                let Kind::Segmented { options, selected } = &self.items[i].kind else {
                    unreachable!("options belong to a segmented toggle");
                };
                let area = laid[i]
                    .parts
                    .iter()
                    .find(|(p, _)| *p == Part::Segment(k))
                    .map_or(Area::None, |&(_, r)| Area::Form(r));
                let mut node = Node::new(key, Role::Radio, options[k].clone(), area);
                node.id = Some(self.items[i].id);
                node.selected = Some(*selected == k);
                node.set = Some((k + 1, options.len()));
                node
            }
            Key::Menu(i) => {
                let area = self
                    .open_menu(i)
                    .and_then(|rows| union(rows.iter().copied()))
                    .map_or(Area::None, Area::Screen);
                let mut node = Node::new(key, Role::List, String::new(), area);
                node.name = self.item_node(i).name;
                node.id = Some(self.items[i].id);
                node
            }
            Key::Entry(i, k) => {
                let Kind::DropDown {
                    entries, current, ..
                } = &self.items[i].kind
                else {
                    unreachable!("entries belong to a drop-down");
                };
                let area = self
                    .open_menu(i)
                    .and_then(|rows| rows.get(k).copied())
                    .map_or(Area::None, Area::Screen);
                let mut node = Node::new(key, Role::ListItem, entries[k].name.clone(), area);
                node.id = Some(self.items[i].id);
                node.help = entries[k].note.clone().unwrap_or_default();
                node.selected = Some(*current == Some(k));
                node.set = Some((k + 1, entries.len()));
                node
            }
        }
    }

    /// The open menu's entries on screen, when the menu is drop-down `i`'s.
    fn open_menu(&self, i: usize) -> Option<&[RECT]> {
        self.form.menu().filter(|m| m.item == i).and(self.menu)
    }

    fn item_node(&self, i: usize) -> Node {
        let item = &self.items[i];
        let look = &self.looks[i];
        let key = Key::Item(i);
        let area = Area::Form(self.rect(i));
        let role = match &item.kind {
            Kind::Card { .. } => Role::Group,
            Kind::Text { .. }
            | Kind::Spans { .. }
            | Kind::Glyph(_)
            | Kind::Band
            | Kind::Rule
            | Kind::Dot { .. } => Role::Text,
            Kind::Button { .. } => Role::Button,
            Kind::Switch { .. } => Role::Switch,
            Kind::Checkbox { .. } | Kind::Day { .. } => Role::CheckBox,
            Kind::Stepper { .. } | Kind::Time { .. } => Role::Spinner,
            Kind::Segmented { .. } => Role::RadioGroup,
            Kind::DropDown { .. } => Role::ComboBox,
            Kind::Choice { .. } => Role::Radio,
        };
        let mut node = Node::new(key, role, String::new(), area);
        if item.kind.focusable() {
            node.id = Some(item.id);
        }
        match &item.kind {
            Kind::Card { .. } => {
                let name = self.card_name(i);
                self.named_by(&mut node, name);
            }
            Kind::Text { style, .. } => {
                node.name = self.text(i);
                let names_group = (0..self.items.len()).any(|g| match self.items[g].kind {
                    Kind::Card { .. } => self.card_name(g) == Some(i),
                    Kind::Choice { .. } => self.heading_before(g) == Some(i),
                    _ => false,
                });
                node.heading = match style {
                    Style::TitleLg => 1,
                    _ if names_group => 2,
                    _ => 0,
                };
            }
            Kind::Spans { .. } => node.name = self.text(i),
            _ => {
                match self.own_label(i) {
                    Some(label) => node.name = label,
                    None => {
                        // A segmented toggle names itself by its segments rather than borrow
                        // its row's name, which its row group already has.
                        let segmented = matches!(item.kind, Kind::Segmented { .. });
                        let text = self.label[i]
                            .or_else(|| if segmented { None } else { self.fallback(i) });
                        self.named_by(&mut node, text);
                    }
                }
                if let Some(t) = (0..self.items.len()).find(|&t| self.note_for[t] == Some(i)) {
                    node.described_by = Some(Key::Item(t));
                    node.help = self.text(t);
                }
            }
        }
        match &item.kind {
            Kind::Switch { on } | Kind::Checkbox { on, .. } | Kind::Day { on, .. } => {
                node.toggle = Some(*on);
            }
            Kind::Stepper {
                value,
                min,
                max,
                step,
                unit,
            } => {
                if !unit.is_empty() {
                    node.name = format!("{} ({})", node.name, unit_words(unit));
                }
                node.value = Some(look.edit.clone().unwrap_or_else(|| value.to_string()));
                node.range = Some(Range {
                    value: f64::from(*value),
                    min: f64::from(*min),
                    max: f64::from(*max),
                    small: f64::from(*step),
                    large: f64::from(step * PAGE_STEPS),
                });
            }
            Kind::Time { minutes, step } => {
                node.time = true;
                node.value = Some(look.edit.clone().unwrap_or_else(|| time_text(*minutes)));
                node.range = Some(Range {
                    value: f64::from(*minutes),
                    min: 0.0,
                    max: f64::from(DAY - 1),
                    small: f64::from(*step),
                    large: f64::from(step * PAGE_STEPS),
                });
            }
            Kind::Segmented { options, .. } if node.name.is_empty() => {
                // No words on its line: the segments' own words name it, "Soft / Firm".
                node.name = options.join(" / ");
            }
            Kind::DropDown {
                entries,
                current,
                other,
            } => {
                let shown = current
                    .and_then(|k| entries.get(k))
                    .map_or(other.clone(), |e| e.name.clone());
                node.value = Some(shown);
                node.expanded = Some(look.open);
            }
            Kind::Choice {
                group,
                note,
                selected,
                ..
            } => {
                node.help = one_line(note);
                node.selected = Some(*selected);
                let members = self.members(*group);
                let at = members.iter().position(|&m| m == i).unwrap_or(0);
                node.set = Some((at + 1, members.len()));
            }
            _ => {}
        }
        node
    }
}

// ---- What a client may do ----

/// Something an automation client asks of an element, done as the user would do it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Move the keyboard focus here.
    Focus(Key),
    /// Press a button.
    Invoke(Key),
    /// Flip a switch, checkbox or day pill.
    Toggle(Key),
    /// Select a segment, a selectable card or a menu entry.
    Select(Key),
    /// Set a stepper's number, or a time stepper's minutes after midnight.
    SetNumber(Key, i32),
    /// Type into a stepper: a number, or a time such as `8:30`.
    Type(Key, String),
    Expand(Key),
    Collapse(Key),
}

/// Does `action` to `form` and returns the events the user would have caused, or `None` when
/// the element cannot do it. The caller hands the events on and repaints.
pub fn perform(form: &mut Form, action: &Action) -> Option<Vec<Event>> {
    let key = match action {
        Action::Focus(k)
        | Action::Invoke(k)
        | Action::Toggle(k)
        | Action::Select(k)
        | Action::SetNumber(k, _)
        | Action::Type(k, _)
        | Action::Expand(k)
        | Action::Collapse(k) => *k,
    };
    let i = key.item()?;
    let item = form.items().get(i)?;
    let id = item.id;
    let kind = item.kind.clone();
    let changed = |form: &mut Form, value: Value| {
        if form.value(id) == Some(value) {
            Some(Vec::new())
        } else {
            form.set(id, value)
                .then(|| vec![Event::Changed(id, form.value(id).unwrap_or(value))])
        }
    };
    match (action, key, &kind) {
        (Action::Focus(_), Key::Entry(_, k), Kind::DropDown { .. }) => {
            let out = form.focus(id);
            if form.menu().is_some_and(|m| m.item == i) {
                form.menu_hover(Some(k));
            }
            Some(out)
        }
        (Action::Focus(_), Key::Item(_) | Key::Option(..), kind) if kind.focusable() => {
            Some(form.focus(id))
        }
        (Action::Invoke(_), Key::Item(_), Kind::Button { .. }) => Some(vec![Event::Pressed(id)]),
        (
            Action::Toggle(_),
            Key::Item(_),
            Kind::Switch { on } | Kind::Checkbox { on, .. } | Kind::Day { on, .. },
        ) => changed(form, Value::On(!on)),
        (Action::Select(_), Key::Item(_), Kind::Choice { .. }) => changed(form, Value::On(true)),
        (Action::Select(_), Key::Option(_, k), Kind::Segmented { options, .. })
            if k < options.len() =>
        {
            changed(form, Value::Index(Some(k)))
        }
        (Action::Select(_), Key::Entry(_, k), Kind::DropDown { entries, .. })
            if k < entries.len() =>
        {
            if form.menu().is_some_and(|m| m.item == i) {
                form.close_menu();
            }
            changed(form, Value::Index(Some(k)))
        }
        (Action::SetNumber(_, v), Key::Item(_), Kind::Stepper { .. }) => {
            changed(form, Value::Number(*v))
        }
        (Action::SetNumber(_, v), Key::Item(_), Kind::Time { .. }) => {
            changed(form, Value::Number(v.rem_euclid(DAY)))
        }
        (Action::Type(_, text), Key::Item(_), Kind::Stepper { .. }) => {
            changed(form, Value::Number(parse_number(text.trim())?))
        }
        (Action::Type(_, text), Key::Item(_), Kind::Time { .. }) => {
            changed(form, Value::Number(parse_time(text.trim())?))
        }
        (Action::Expand(_), Key::Item(_), Kind::DropDown { .. }) => {
            let mut out = form.focus(id);
            if form.menu().is_none_or(|m| m.item != i) {
                out.extend(form.key(super::super::form::Key::AltDown));
            }
            Some(out)
        }
        (Action::Collapse(_), Key::Item(_), Kind::DropDown { .. }) => {
            if form.menu().is_some_and(|m| m.item == i) {
                form.close_menu();
            }
            Some(Vec::new())
        }
        _ => None,
    }
}

// ---- What changed ----

/// A change a screen reader hears about.
#[derive(Clone, Debug, PartialEq)]
pub enum Change {
    Name(Key, String, String),
    Toggle(Key, bool, bool),
    Range(Key, f64, f64),
    Value(Key, String, String),
    Expanded(Key, bool, bool),
    /// Selected or no longer; a newly selected element also raises its selected event.
    Selected(Key, bool, bool),
    /// The focus moved here.
    Focus(Key),
}

/// What changed from `before` to `after`: property changes in tree order, then the focus.
pub fn changes(before: &Tree, after: &Tree) -> Vec<Change> {
    let mut out = Vec::new();
    for n in &after.nodes {
        let Some(b) = before.get(n.key) else {
            continue;
        };
        let k = n.key;
        if b.name != n.name {
            out.push(Change::Name(k, b.name.clone(), n.name.clone()));
        }
        if let (Some(x), Some(y)) = (b.toggle, n.toggle)
            && x != y
        {
            out.push(Change::Toggle(k, x, y));
        }
        if let (Some(x), Some(y)) = (b.range, n.range)
            && x.value != y.value
        {
            out.push(Change::Range(k, x.value, y.value));
        }
        if let (Some(x), Some(y)) = (&b.value, &n.value)
            && x != y
        {
            out.push(Change::Value(k, x.clone(), y.clone()));
        }
        if let (Some(x), Some(y)) = (b.expanded, n.expanded)
            && x != y
        {
            out.push(Change::Expanded(k, x, y));
        }
        if let (Some(x), Some(y)) = (b.selected, n.selected)
            && x != y
        {
            out.push(Change::Selected(k, x, y));
        }
    }
    if let Some(f) = after.focus
        && before.focus != after.focus
    {
        out.push(Change::Focus(f));
    }
    out
}
