//! The accessible trees of the settings window and the wizard, what automation clients may do
//! to them, the events they raise, and the providers UI Automation calls, all without a screen
//! reader. The provider tests run against a fake window over a bare form.

use std::cell::RefCell;

use breakpoint::core::config::Config;
use breakpoint::core::settings::Form as Settings;
use breakpoint::core::wizard::Picks;
use windows::Win32::Foundation::{E_INVALIDARG, HWND, RECT};
use windows::Win32::Graphics::Gdi::{GetDC, ReleaseDC};
use windows::Win32::System::Ole::{SafeArrayDestroy, SafeArrayGetElement, SafeArrayGetUBound};
use windows::Win32::System::Variant::{VARIANT, VT_ARRAY, VT_BOOL, VT_BSTR, VT_I4, VT_UNKNOWN};
use windows::Win32::UI::WindowsAndMessaging::{KillTimer, SetTimer};
use windows::core::{IUnknown, Interface};

use super::tree::perform;
use super::*;
use crate::platform::kit::draw::GdiMeasure;
use crate::platform::kit::form::{self, Form, Measure, rect};
use crate::platform::kit::{Event, Item, Value};
use crate::platform::theme::{Fonts, Style, px};
use crate::platform::{dialogs, settings_window};

const SCALES: [u32; 4] = [96, 120, 144, 192];

/// Every character 7px wide at 96 DPI, scaled with the DPI, at any style.
struct Fixed(u32);

impl Measure for Fixed {
    fn width(&self, _: Style, text: &str) -> i32 {
        px(7 * text.chars().count() as i32, self.0)
    }
}

fn laid(items: Vec<Item>, dpi: u32) -> Form {
    let mut form = Form::new(items);
    form.lay_out(dpi, &Fixed(dpi));
    form
}

/// Laid out with the real fonts, as the window does.
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

fn settings_items() -> Vec<Item> {
    settings_window::items(&Settings::new(&Config::default()), true)
}

fn wizard_items() -> Vec<Item> {
    dialogs::items(&Picks::default())
}

fn settings() -> Form {
    laid(settings_items(), 96)
}

fn wizard() -> Form {
    laid(wizard_items(), 96)
}

fn tree(form: &Form) -> Tree {
    Tree::new(form, None)
}

/// The `n`th element with `role` and `name`, in tree order.
fn nth(tree: &Tree, role: Role, name: &str, n: usize) -> Key {
    tree.nodes()
        .iter()
        .filter(|node| node.role == role && node.name == name)
        .nth(n)
        .unwrap_or_else(|| panic!("no {role:?} {name:?} number {n}"))
        .key
}

fn find(tree: &Tree, role: Role, name: &str) -> Key {
    nth(tree, role, name, 0)
}

fn id_of(tree: &Tree, key: Key) -> u16 {
    tree.get(key).and_then(|n| n.id).expect("a control")
}

const SETTINGS: &str = r#"window
  text "Settings" h1
  text "Nudges fall due only inside your work week."
  text "Work week" h2
  group "Work week"
    group "Work days"
      text "Work days"
      checkbox "Sun" on focused
      checkbox "Mon" on
      checkbox "Tue" on
      checkbox "Wed" on
      checkbox "Thu" on
      checkbox "Fri" off
      checkbox "Sat" off
    group "Work hours"
      text "Work hours"
      spinner "Work hours" ="08:00" [0..=1439 by 15]
      text "to"
      spinner "to" ="17:00" [0..=1439 by 15]
  text "Nudges" h2
  group "Nudges"
    text "Profile"
    text "Fills the numbers below."
    combobox "Profile" ="Balanced" closed -- "Fills the numbers below."
      list "Profile"
        listitem "Gentle" 1/3 -- "Stand every 45 min, pause every 90"
        listitem "Balanced" selected 2/3 -- "Stand every 30 min, pause every 60 (Pause insists)"
        listitem "Strict" 3/3 -- "Stand every 30 min, pause every 30 (both insist)"
    group "Stand up and move"
      text "Stand up and move"
      switch "Stand up and move" on
      text "Every"
      spinner "Every (minutes)" ="30" [10..=240 by 5]
      text "Away for"
      spinner "Away for (minutes)" ="2" [2..=5 by 1]
      radiogroup "Soft / Firm"
        radio "Soft" selected 1/2
        radio "Firm" 2/2
    group "Pause"
      text "Pause"
      switch "Pause" on
      text "Every"
      spinner "Every (minutes)" ="60" [20..=240 by 5]
      text "Away for"
      spinner "Away for (minutes)" ="5" [2..=15 by 1]
      radiogroup "Soft / Firm"
        radio "Soft" 1/2
        radio "Firm" selected 2/2
    group "Drink some water"
      text "Drink some water"
      switch "Drink some water" on
      text "At most every"
      spinner "At most every (minutes)" ="60" [10..=240 by 5] -- "Always gentle; rides along with the next nudge."
      text "Always gentle; rides along with the next nudge."
  text "General" h2
  group "General"
    text "Start with Windows"
    text "breakpoint starts in the tray when you sign in."
    switch "Start with Windows" on -- "breakpoint starts in the tray when you sign in."
  button "Save"
  button "Cancel"
"#;

const WIZARD: &str = r#"window
  text "Pick a rhythm" h1
  text "You can change any of it later in settings."
  radiogroup "Pick a rhythm"
    radio "Gentle" 1/3 -- "Stand every 45 min, pause every 90. Water rides along hourly."
    radio "Balanced" selected 2/3 focused -- "Stand every 30 min, pause every 60 (Pause insists). Water rides along hourly."
    radio "Strict" 3/3 -- "Stand every 30 min, pause every 30 (both insist). Water rides along hourly."
  group "Work week"
    text "Work week" h2
    text "Nudges Sunday to Thursday, 08:00 to 17:00"
    button "Change…" -- "Nudges Sunday to Thursday, 08:00 to 17:00"
  checkbox "Start with Windows" on -- "breakpoint lives in the tray, under ^ if you don't see it."
  text "breakpoint lives in the tray, under ^ if you don't see it."
  button "Start"
"#;

// ---- The trees ----

#[test]
fn the_settings_window_reads_as_cards_of_named_controls() {
    assert_eq!(tree(&settings()).outline(), SETTINGS);
}

#[test]
fn the_wizard_reads_as_a_radio_group_of_profiles_and_its_buttons() {
    assert_eq!(tree(&wizard()).outline(), WIZARD);
}

/// The names come from where the labels sit, so they must survive the real fonts at every
/// scale, where texts and controls move by a pixel or wrap.
#[test]
fn both_trees_read_the_same_at_every_scale_with_the_real_fonts() {
    for dpi in SCALES {
        let settings = Tree::new(&laid_for_real(settings_items(), dpi), None);
        assert_eq!(settings.outline(), SETTINGS, "settings at {dpi} DPI");
        let wizard = Tree::new(&laid_for_real(wizard_items(), dpi), None);
        assert_eq!(wizard.outline(), WIZARD, "wizard at {dpi} DPI");
    }
}

/// The rules Accessibility Insights checks on structure: a name on everything that takes the
/// focus, no two focusable siblings alike, no child named and typed as its parent, and every
/// child inside its parent's rectangle.
#[test]
fn both_trees_keep_the_structure_rules_at_every_scale() {
    for dpi in SCALES {
        for (what, items) in [("settings", settings_items()), ("wizard", wizard_items())] {
            let tree = Tree::new(&laid_for_real(items, dpi), None);
            for node in tree.nodes() {
                let at = format!("{what} at {dpi} DPI: {:?} {:?}", node.role, node.name);
                if node.role.focusable() {
                    assert!(!node.name.is_empty(), "{at} has no name");
                }
                if node.role != Role::Text && node.role != Role::Root {
                    assert!(!node.name.is_empty(), "{at} has no name");
                }
                let Some(parent) = node.parent.and_then(|p| tree.get(p)) else {
                    continue;
                };
                assert!(
                    !(parent.name == node.name && parent.role == node.role),
                    "{at} is named and typed as its parent"
                );
                let twins = parent
                    .children
                    .iter()
                    .filter_map(|&k| tree.get(k))
                    .filter(|s| s.role.focusable() && s.role == node.role && s.name == node.name)
                    .count();
                assert!(twins <= 1, "{at} has a twin");
                if let (Area::Form(outer), Area::Form(inner)) = (parent.area, node.area) {
                    assert!(
                        inner.left >= outer.left
                            && inner.top >= outer.top
                            && inner.right <= outer.right
                            && inner.bottom <= outer.bottom,
                        "{at} at {inner:?} pokes out of its parent at {outer:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn the_focus_follows_the_segment_and_the_highlighted_menu_entry() {
    let mut form = settings();
    let t = tree(&form);
    let soft = find(&t, Role::Radio, "Soft");
    let Key::Option(level, 0) = soft else {
        panic!("a segment: {soft:?}");
    };
    form.focus(form.items()[level].id);
    assert_eq!(tree(&form).focus(), Some(soft));

    let profile = find(&t, Role::ComboBox, "Profile");
    let Key::Item(p) = profile else {
        panic!("an item: {profile:?}");
    };
    form.focus(form.items()[p].id);
    form.key(form::Key::Space);
    let rows = [
        rect(0, 0, 100, 40),
        rect(0, 40, 100, 40),
        rect(0, 80, 100, 40),
    ];
    let open = Tree::new(&form, Some(&rows));
    assert_eq!(open.focus(), Some(Key::Entry(p, 1)), "the current entry");
    assert_eq!(
        open.get(Key::Entry(p, 2)).unwrap().area,
        Area::Screen(rows[2])
    );
    assert_eq!(
        open.get(Key::Menu(p)).unwrap().area,
        Area::Screen(rect(0, 0, 100, 120))
    );
    assert_eq!(open.get(profile).unwrap().expanded, Some(true));
    let closed = tree(&settings());
    assert_eq!(closed.get(Key::Menu(p)).unwrap().area, Area::None);
}

#[test]
fn typed_text_is_the_value_while_typing() {
    let mut form = settings();
    let every = find(&tree(&form), Role::Spinner, "Every (minutes)");
    form.focus(id_of(&tree(&form), every));
    form.key(form::Key::Char('4'));
    let t = tree(&form);
    assert_eq!(t.get(every).unwrap().value.as_deref(), Some("4"));
    assert_eq!(t.get(every).unwrap().range.unwrap().value, 4.0);
}

// ---- Actions ----

fn act(form: &mut Form, action: Action) -> Option<Vec<Event>> {
    perform(form, &action)
}

#[test]
fn actions_change_the_form_as_the_user_would() {
    let mut form = settings();
    let t = tree(&form);

    let fri = find(&t, Role::CheckBox, "Fri");
    let id = id_of(&t, fri);
    assert_eq!(
        act(&mut form, Action::Toggle(fri)),
        Some(vec![Event::Changed(id, Value::On(true))])
    );
    assert_eq!(tree(&form).get(fri).unwrap().toggle, Some(true));

    let firm = find(&t, Role::Radio, "Firm");
    let level = id_of(&t, firm);
    assert_eq!(
        act(&mut form, Action::Select(firm)),
        Some(vec![Event::Changed(level, Value::Index(Some(1)))])
    );
    assert_eq!(
        act(&mut form, Action::Select(firm)),
        Some(vec![]),
        "already selected"
    );

    let every = find(&t, Role::Spinner, "Every (minutes)");
    let id = id_of(&t, every);
    assert_eq!(
        act(&mut form, Action::SetNumber(every, 45)),
        Some(vec![Event::Changed(id, Value::Number(45))])
    );
    assert_eq!(
        act(&mut form, Action::Type(every, "50".into())),
        Some(vec![Event::Changed(id, Value::Number(50))])
    );
    assert_eq!(act(&mut form, Action::Type(every, "soon".into())), None);

    let from = find(&t, Role::Spinner, "Work hours");
    let id = id_of(&t, from);
    assert_eq!(
        act(&mut form, Action::Type(from, "8:30".into())),
        Some(vec![Event::Changed(id, Value::Number(8 * 60 + 30))])
    );
    assert_eq!(
        act(&mut form, Action::SetNumber(from, -15)),
        Some(vec![Event::Changed(id, Value::Number(form::DAY - 15))]),
        "times wrap round midnight"
    );

    let save = find(&t, Role::Button, "Save");
    assert_eq!(
        act(&mut form, Action::Invoke(save)),
        Some(vec![Event::Pressed(id_of(&t, save))])
    );

    let title = find(&t, Role::Text, "Settings");
    assert_eq!(act(&mut form, Action::Toggle(title)), None);
    assert_eq!(act(&mut form, Action::Invoke(fri)), None);
    assert_eq!(act(&mut form, Action::Focus(title)), None);

    act(&mut form, Action::Focus(every)).unwrap();
    assert_eq!(form.focused(), Some(id_of(&t, every)));
}

#[test]
fn the_drop_down_opens_picks_and_closes() {
    let mut form = settings();
    let t = tree(&form);
    let profile = find(&t, Role::ComboBox, "Profile");
    let Key::Item(p) = profile else {
        panic!("an item");
    };
    let id = id_of(&t, profile);

    act(&mut form, Action::Expand(profile)).unwrap();
    assert_eq!(form.menu().map(|m| m.item), Some(p));
    assert_eq!(form.focused(), Some(id));
    act(&mut form, Action::Expand(profile)).unwrap();
    assert_eq!(form.menu().map(|m| m.item), Some(p), "stays open");

    act(&mut form, Action::Focus(Key::Entry(p, 2))).unwrap();
    assert_eq!(form.menu().and_then(|m| m.hot), Some(2));

    assert_eq!(
        act(&mut form, Action::Select(Key::Entry(p, 2))),
        Some(vec![Event::Changed(id, Value::Index(Some(2)))])
    );
    assert_eq!(form.menu(), None, "picking closes the menu");

    act(&mut form, Action::Expand(profile)).unwrap();
    act(&mut form, Action::Collapse(profile)).unwrap();
    assert_eq!(form.menu(), None);
}

#[test]
fn selecting_a_profile_card_selects_it_alone() {
    let mut form = wizard();
    let t = tree(&form);
    let strict = find(&t, Role::Radio, "Strict");
    let balanced = find(&t, Role::Radio, "Balanced");
    assert_eq!(
        act(&mut form, Action::Select(strict)),
        Some(vec![Event::Changed(id_of(&t, strict), Value::On(true))])
    );
    let after = tree(&form);
    assert_eq!(after.get(strict).unwrap().selected, Some(true));
    assert_eq!(after.get(balanced).unwrap().selected, Some(false));
    assert_eq!(after.selection(Key::Choices(1)), vec![strict]);
}

// ---- Changes ----

#[test]
fn a_toggle_raises_its_state_change_only() {
    let mut form = settings();
    let before = tree(&form);
    let fri = find(&before, Role::CheckBox, "Fri");
    act(&mut form, Action::Toggle(fri));
    assert_eq!(
        changes(&before, &tree(&form)),
        vec![Change::Toggle(fri, false, true)]
    );
}

#[test]
fn an_arrow_in_the_profile_cards_moves_the_selection_and_the_focus() {
    let mut form = wizard();
    let before = tree(&form);
    let balanced = find(&before, Role::Radio, "Balanced");
    let strict = find(&before, Role::Radio, "Strict");
    form.key(form::Key::Down);
    assert_eq!(
        changes(&before, &tree(&form)),
        vec![
            Change::Selected(balanced, true, false),
            Change::Selected(strict, false, true),
            Change::Focus(strict),
        ]
    );
}

#[test]
fn opening_the_menu_expands_and_moves_the_focus_to_the_current_entry() {
    let mut form = settings();
    let profile = find(&tree(&form), Role::ComboBox, "Profile");
    let Key::Item(p) = profile else {
        panic!("an item");
    };
    form.focus(form.items()[p].id);
    let before = tree(&form);
    form.key(form::Key::Space);
    assert_eq!(
        changes(&before, &tree(&form)),
        vec![
            Change::Expanded(profile, false, true),
            Change::Focus(Key::Entry(p, 1)),
        ]
    );
}

#[test]
fn a_profile_pick_and_new_text_raise_value_and_name_changes() {
    let mut form = settings();
    let before = tree(&form);
    let profile = find(&before, Role::ComboBox, "Profile");
    let id = id_of(&before, profile);
    form.set(id, Value::Index(None));
    form.set_text(id, "Mine");
    let after = tree(&form);
    let found = changes(&before, &after);
    assert!(
        found.contains(&Change::Value(profile, "Balanced".into(), "Mine".into())),
        "{found:?}"
    );
    let Key::Item(p) = profile else {
        panic!("an item");
    };
    assert!(found.contains(&Change::Selected(Key::Entry(p, 1), true, false)));

    let mut form = wizard();
    let before = tree(&form);
    let line = find(
        &before,
        Role::Text,
        "Nudges Sunday to Thursday, 08:00 to 17:00",
    );
    let Key::Item(l) = line else {
        panic!("an item");
    };
    let id = form.items()[l].id;
    // The wizard's texts share one id; the first text is the title.
    assert!(form.set_text(id, "Mon to Fri"));
    let found = changes(&before, &tree(&form));
    assert!(matches!(found.first(), Some(Change::Name(..))), "{found:?}");
}

// ---- The providers ----

thread_local! {
    static FORM: RefCell<Option<Form>> = const { RefCell::new(None) };
    static EVENTS: RefCell<Vec<Event>> = const { RefCell::new(Vec::new()) };
}

/// Where the fake window's client area starts on screen.
const ORIGIN: (i32, i32) = (1000, 500);

/// A window over this thread's [`FORM`], its client area at [`ORIGIN`], always focused.
/// `hosted`: a real window on a thread of its own, which owns the form; else no window at all,
/// the test thread acts as the window's, and a thread timer wakes it for posted actions.
struct Fake {
    hosted: bool,
}

impl Source for Fake {
    fn tree(&self, _: HWND) -> Option<Tree> {
        FORM.with_borrow(|f| f.as_ref().map(|f| Tree::new(f, None)))
    }

    fn act(&self, _: HWND, action: Action) -> bool {
        let events = FORM.with_borrow_mut(|f| f.as_mut().and_then(|f| perform(f, &action)));
        let done = events.is_some();
        EVENTS.with_borrow_mut(|e| e.extend(events.into_iter().flatten()));
        done
    }

    fn to_screen(&self, _: HWND, r: RECT) -> RECT {
        RECT {
            left: r.left + ORIGIN.0,
            top: r.top + ORIGIN.1,
            right: r.right + ORIGIN.0,
            bottom: r.bottom + ORIGIN.1,
        }
    }

    fn view(&self, _: HWND) -> RECT {
        rect(ORIGIN.0, ORIGIN.1, 2000, 2000)
    }

    fn has_focus(&self, _: HWND) -> bool {
        true
    }

    fn on_thread(&self, hwnd: HWND) -> bool {
        if self.hosted {
            window_thread(hwnd)
        } else {
            FORM.with_borrow(Option::is_some)
        }
    }

    fn wake(&self, hwnd: HWND, wait: bool) {
        if self.hosted {
            wake_window(hwnd, wait);
        } else {
            // SAFETY: a thread timer with no window; it kills itself.
            unsafe { SetTimer(None, 0, 0, Some(wake_fake)) };
        }
    }
}

unsafe extern "system" fn wake_fake(_: HWND, _: u32, id: usize, _: u32) {
    // SAFETY: the thread timer that called.
    let _ = unsafe { KillTimer(None, id) };
    run_posted(HWND::default());
}

static FAKE: Fake = Fake { hosted: false };
static HOSTED: Fake = Fake { hosted: true };

/// The root provider of a fake window over `form`.
fn root(form: Form) -> IRawElementProviderFragmentRoot {
    FORM.set(Some(form));
    EVENTS.take();
    Element::new(HWND::default(), &FAKE, Key::Root).into_interface()
}

fn provider(key: Key) -> IRawElementProviderSimple {
    Element::new(HWND::default(), &FAKE, key).into_interface()
}

/// What a VARIANT holds, read and cleared.
#[derive(Debug, PartialEq)]
enum Got {
    Empty,
    Str(String),
    Int(i32),
    Bool(bool),
    Unknown(IUnknown),
    Unknowns(Vec<IUnknown>),
}

fn got(v: VARIANT) -> Got {
    // Cleared when it goes, after everything is copied out.
    let v = Var(v);
    // SAFETY: the tag says which field is live.
    unsafe {
        let inner = &*v.0.Anonymous.Anonymous;
        match inner.vt {
            VT_BSTR => Got::Str(inner.Anonymous.bstrVal.to_string()),
            VT_I4 => Got::Int(inner.Anonymous.lVal),
            VT_BOOL => Got::Bool(inner.Anonymous.boolVal.as_bool()),
            VT_UNKNOWN => Got::Unknown((*inner.Anonymous.punkVal).clone().unwrap()),
            vt if vt == VT_ARRAY | VT_UNKNOWN => {
                Got::Unknowns(unknowns(inner.Anonymous.parray, false))
            }
            _ => Got::Empty,
        }
    }
}

/// The interfaces in a SAFEARRAY; destroys it when `own`.
fn unknowns(array: *mut SAFEARRAY, own: bool) -> Vec<IUnknown> {
    if array.is_null() {
        return Vec::new();
    }
    // SAFETY: a one-dimensional array of interface pointers; each read adds a reference.
    unsafe {
        let last = SafeArrayGetUBound(array, 1).unwrap();
        let out = (0..=last)
            .map(|at| {
                let mut p = std::ptr::null_mut();
                SafeArrayGetElement(array, &at, &mut p as *mut _ as _).unwrap();
                IUnknown::from_raw(p)
            })
            .collect();
        if own {
            SafeArrayDestroy(array).unwrap();
        }
        out
    }
}

fn ints(array: *mut SAFEARRAY) -> Vec<i32> {
    // SAFETY: a one-dimensional array of four-byte integers, destroyed after.
    unsafe {
        let last = SafeArrayGetUBound(array, 1).unwrap();
        let out = (0..=last)
            .map(|at| {
                let mut v = 0i32;
                SafeArrayGetElement(array, &at, &mut v as *mut i32 as _).unwrap();
                v
            })
            .collect();
        SafeArrayDestroy(array).unwrap();
        out
    }
}

fn prop(p: &IRawElementProviderSimple, id: UIA_PROPERTY_ID) -> Got {
    // SAFETY: a live provider.
    got(unsafe { p.GetPropertyValue(id) }.unwrap())
}

fn name(p: &impl Interface) -> String {
    match prop(&p.cast().unwrap(), UIA_NamePropertyId) {
        Got::Str(s) => s,
        other => panic!("no name: {other:?}"),
    }
}

/// Every element under `from` in tree order, by walking the providers.
fn walk(from: &IRawElementProviderFragment, out: &mut Vec<(i32, String)>) {
    // SAFETY: live providers; a missing sibling ends the walk.
    unsafe {
        let mut child = from.Navigate(NavigateDirection_FirstChild).ok();
        while let Some(c) = child {
            let simple: IRawElementProviderSimple = c.cast().unwrap();
            let Got::Int(control) = prop(&simple, UIA_ControlTypePropertyId) else {
                panic!("no control type");
            };
            out.push((control, name(&simple)));
            walk(&c, out);
            child = c.Navigate(NavigateDirection_NextSibling).ok();
        }
    }
}

#[test]
fn the_provider_tree_walks_as_the_tree_reads() {
    let form = settings();
    let expected: Vec<(i32, String)> = tree(&form)
        .nodes()
        .iter()
        .skip(1)
        .map(|n| (Element::control_type(n.role).0.0, n.name.clone()))
        .collect();
    let root = root(form);
    let mut walked = Vec::new();
    walk(&root.cast().unwrap(), &mut walked);
    // The tree's nodes are in creation order, which is reading order but parents first;
    // compare as sets of the same size.
    assert_eq!(walked.len(), expected.len());
    for e in &expected {
        assert!(walked.contains(e), "{e:?} missing from the walk");
    }
    // Parent and back.
    // SAFETY: live providers.
    unsafe {
        let first = root
            .cast::<IRawElementProviderFragment>()
            .unwrap()
            .Navigate(NavigateDirection_FirstChild)
            .unwrap();
        let parent = first.Navigate(NavigateDirection_Parent).unwrap();
        assert_eq!(
            parent.GetRuntimeId().unwrap(),
            std::ptr::null_mut(),
            "the root"
        );
        assert!(first.Navigate(NavigateDirection_PreviousSibling).is_err());
    }
}

#[test]
fn the_providers_give_names_types_and_relations() {
    let form = settings();
    let t = tree(&form);
    let root = root(form);
    let switch = provider(find(&t, Role::Switch, "Start with Windows"));
    assert_eq!(
        prop(&switch, UIA_ControlTypePropertyId),
        Got::Int(UIA_CheckBoxControlTypeId.0)
    );
    assert_eq!(
        prop(&switch, UIA_LocalizedControlTypePropertyId),
        Got::Str("toggle switch".into())
    );
    assert_eq!(
        prop(&switch, UIA_IsKeyboardFocusablePropertyId),
        Got::Bool(true)
    );
    assert_eq!(
        prop(&switch, UIA_HasKeyboardFocusPropertyId),
        Got::Bool(false)
    );
    assert_eq!(prop(&switch, UIA_IsOffscreenPropertyId), Got::Bool(false));
    assert_eq!(
        prop(&switch, UIA_HelpTextPropertyId),
        Got::Str("breakpoint starts in the tray when you sign in.".into())
    );
    match prop(&switch, UIA_LabeledByPropertyId) {
        Got::Unknown(label) => assert_eq!(name(&label), "Start with Windows"),
        other => panic!("no label: {other:?}"),
    }
    match prop(&switch, UIA_DescribedByPropertyId) {
        Got::Unknowns(notes) => {
            assert_eq!(notes.len(), 1);
            assert_eq!(
                name(&notes[0]),
                "breakpoint starts in the tray when you sign in."
            );
        }
        other => panic!("no note: {other:?}"),
    }

    let sun = provider(find(&t, Role::CheckBox, "Sun"));
    assert_eq!(prop(&sun, UIA_HasKeyboardFocusPropertyId), Got::Bool(true));
    assert_eq!(prop(&sun, UIA_LocalizedControlTypePropertyId), Got::Empty);
    let heading = provider(find(&t, Role::Text, "Work week"));
    assert_eq!(
        prop(&heading, UIA_HeadingLevelPropertyId),
        Got::Int(HeadingLevel2.0)
    );
    let firm = provider(nth(&t, Role::Radio, "Firm", 0));
    assert_eq!(prop(&firm, UIA_PositionInSetPropertyId), Got::Int(2));
    assert_eq!(prop(&firm, UIA_SizeOfSetPropertyId), Got::Int(2));

    // SAFETY: live providers.
    unsafe {
        let focus = root.GetFocus().unwrap();
        assert_eq!(name(&focus), "Sun");
        let id = ints(focus.GetRuntimeId().unwrap());
        assert_eq!(id[0], UiaAppendRuntimeId as i32);
        let sun_rect = t.get(find(&t, Role::CheckBox, "Sun")).unwrap().area;
        let Area::Form(r) = sun_rect else {
            panic!("on the form");
        };
        let b = focus.BoundingRectangle().unwrap();
        assert_eq!(
            (b.left, b.top),
            (f64::from(r.left + ORIGIN.0), f64::from(r.top + ORIGIN.1))
        );
        let hit = root
            .ElementProviderFromPoint(b.left + b.width / 2.0, b.top + b.height / 2.0)
            .unwrap();
        assert_eq!(name(&hit), "Sun");
        let nowhere = root.ElementProviderFromPoint(0.0, 0.0).unwrap();
        assert_eq!(
            nowhere.GetRuntimeId().unwrap(),
            std::ptr::null_mut(),
            "the root"
        );
    }
}

fn pattern<I: Interface>(p: &IRawElementProviderSimple, id: UIA_PATTERN_ID) -> Option<I> {
    // SAFETY: a live provider.
    unsafe { p.GetPatternProvider(id) }
        .ok()
        .map(|u| u.cast().unwrap())
}

#[test]
fn each_control_offers_its_patterns_only() {
    let form = settings();
    let t = tree(&form);
    let _root = root(form);
    let offers = |role, name: &str| {
        let p = provider(find(&t, role, name));
        [
            UIA_InvokePatternId,
            UIA_TogglePatternId,
            UIA_SelectionPatternId,
            UIA_SelectionItemPatternId,
            UIA_RangeValuePatternId,
            UIA_ValuePatternId,
            UIA_ExpandCollapsePatternId,
        ]
        .map(|id| pattern::<IUnknown>(&p, id).is_some())
    };
    let (n, y) = (false, true);
    //                                                        Inv Tog Sel Item Range Val Exp
    assert_eq!(offers(Role::Button, "Save"), [y, n, n, n, n, n, n]);
    assert_eq!(offers(Role::Switch, "Pause"), [n, y, n, n, n, n, n]);
    assert_eq!(offers(Role::CheckBox, "Mon"), [n, y, n, n, n, n, n]);
    assert_eq!(
        offers(Role::RadioGroup, "Soft / Firm"),
        [n, n, y, n, n, n, n]
    );
    assert_eq!(offers(Role::Radio, "Soft"), [n, n, n, y, n, n, n]);
    assert_eq!(offers(Role::Spinner, "to"), [n, n, n, n, y, y, n]);
    assert_eq!(offers(Role::ComboBox, "Profile"), [n, n, y, n, n, y, y]);
    assert_eq!(offers(Role::List, "Profile"), [n, n, y, n, n, n, n]);
    assert_eq!(offers(Role::ListItem, "Strict"), [n, n, n, y, n, n, n]);
    assert_eq!(offers(Role::Group, "Nudges"), [n; 7]);
    assert_eq!(offers(Role::Text, "Every"), [n; 7]);
}

#[test]
fn the_patterns_read_and_change_the_form() {
    let form = settings();
    let t = tree(&form);
    let _root = root(form);
    // SAFETY: live providers.
    unsafe {
        let fri = provider(find(&t, Role::CheckBox, "Fri"));
        let toggle: IToggleProvider = pattern(&fri, UIA_TogglePatternId).unwrap();
        assert_eq!(toggle.ToggleState().unwrap(), ToggleState_Off);
        toggle.Toggle().unwrap();
        assert_eq!(toggle.ToggleState().unwrap(), ToggleState_On);

        let every = provider(find(&t, Role::Spinner, "Every (minutes)"));
        let range: IRangeValueProvider = pattern(&every, UIA_RangeValuePatternId).unwrap();
        assert_eq!(
            (range.Minimum().unwrap(), range.Maximum().unwrap()),
            (10.0, 240.0)
        );
        assert_eq!(
            (range.SmallChange().unwrap(), range.LargeChange().unwrap()),
            (5.0, 25.0)
        );
        assert_eq!(range.SetValue(500.0).unwrap_err().code(), E_INVALIDARG);
        range.SetValue(45.0).unwrap();
        assert_eq!(range.Value().unwrap(), 45.0);
        let value: IValueProvider = pattern(&every, UIA_ValuePatternId).unwrap();
        assert_eq!(value.Value().unwrap(), "45");
        assert!(!value.IsReadOnly().unwrap().as_bool());
        value.SetValue(windows::core::w!("55")).unwrap();
        assert_eq!(range.Value().unwrap(), 55.0);
        assert_eq!(
            value.SetValue(windows::core::w!("x")).unwrap_err().code(),
            E_INVALIDARG
        );

        let to = provider(find(&t, Role::Spinner, "to"));
        let value: IValueProvider = pattern(&to, UIA_ValuePatternId).unwrap();
        assert_eq!(value.Value().unwrap(), "17:00");
        value.SetValue(windows::core::w!("16:30")).unwrap();
        assert_eq!(value.Value().unwrap(), "16:30");

        let level = provider(nth(&t, Role::RadioGroup, "Soft / Firm", 0));
        let selection: ISelectionProvider = pattern(&level, UIA_SelectionPatternId).unwrap();
        let chosen = unknowns(selection.GetSelection().unwrap(), true);
        assert_eq!(chosen.iter().map(name).collect::<Vec<_>>(), ["Soft"]);
        assert!(selection.IsSelectionRequired().unwrap().as_bool());
        let firm = provider(nth(&t, Role::Radio, "Firm", 0));
        let item: ISelectionItemProvider = pattern(&firm, UIA_SelectionItemPatternId).unwrap();
        assert!(!item.IsSelected().unwrap().as_bool());
        assert!(item.AddToSelection().is_err());
        item.Select().unwrap();
        assert!(item.IsSelected().unwrap().as_bool());
        assert_eq!(name(&item.SelectionContainer().unwrap()), "Soft / Firm");

        let profile = provider(find(&t, Role::ComboBox, "Profile"));
        let expand: IExpandCollapseProvider =
            pattern(&profile, UIA_ExpandCollapsePatternId).unwrap();
        assert_eq!(
            expand.ExpandCollapseState().unwrap(),
            ExpandCollapseState_Collapsed
        );
        expand.Expand().unwrap();
        assert_eq!(
            expand.ExpandCollapseState().unwrap(),
            ExpandCollapseState_Expanded
        );
        let value: IValueProvider = pattern(&profile, UIA_ValuePatternId).unwrap();
        assert!(value.IsReadOnly().unwrap().as_bool());
        let strict = provider(find(&t, Role::ListItem, "Strict"));
        let item: ISelectionItemProvider = pattern(&strict, UIA_SelectionItemPatternId).unwrap();
        item.Select().unwrap();
        assert_eq!(
            expand.ExpandCollapseState().unwrap(),
            ExpandCollapseState_Collapsed
        );
        assert_eq!(value.Value().unwrap(), "Strict");
        let selection: ISelectionProvider = pattern(&profile, UIA_SelectionPatternId).unwrap();
        let chosen = unknowns(selection.GetSelection().unwrap(), true);
        assert_eq!(chosen.iter().map(name).collect::<Vec<_>>(), ["Strict"]);

        let pause = provider(find(&t, Role::Switch, "Pause"));
        pause
            .cast::<IRawElementProviderFragment>()
            .unwrap()
            .SetFocus()
            .unwrap();
    }
    let pause_id = id_of(&t, find(&t, Role::Switch, "Pause"));
    assert_eq!(
        FORM.with_borrow(|f| f.as_ref().unwrap().focused()),
        Some(pause_id)
    );
    let events = EVENTS.take();
    assert!(events.contains(&Event::Changed(
        id_of(&t, find(&t, Role::CheckBox, "Fri")),
        Value::On(true)
    )));
}

/// Invoke returns before the button acts: the press waits for the message loop, as a Save
/// that shows a message box must not hold the screen reader.
#[test]
fn invoke_presses_the_button_after_returning() {
    let form = wizard();
    let t = tree(&form);
    let _root = root(form);
    let start = provider(find(&t, Role::Button, "Start"));
    let invoke: IInvokeProvider = pattern(&start, UIA_InvokePatternId).unwrap();
    // SAFETY: a live provider.
    unsafe { invoke.Invoke() }.unwrap();
    assert_eq!(EVENTS.with_borrow(Vec::len), 0, "not yet");
    crate::platform::kit::shots::pump(100);
    assert_eq!(
        EVENTS.take(),
        [Event::Pressed(id_of(&t, find(&t, Role::Button, "Start")))]
    );
}

#[test]
fn a_gone_window_says_so() {
    let p = provider(Key::Item(3));
    FORM.set(None);
    // SAFETY: a live provider over no form.
    let err = unsafe { p.GetPropertyValue(UIA_NamePropertyId) }
        .err()
        .expect("an error");
    assert_eq!(err.code(), HRESULT(UIA_E_ELEMENTNOTAVAILABLE as i32));
}

mod client;
