//! UI Automation for kit windows (issue #80): painted controls have no window of their own, so
//! the kit tells screen readers what they are.
//!
//! The window answers `WM_GETOBJECT` with [`get_object`], which hands UI Automation the root of
//! a provider tree: the window as the fragment root, one fragment per element of [`Tree`]. Every
//! call reads the form afresh through a [`Source`], so the tree never goes stale. [`changed`],
//! called after anything that may change the form, raises the focus and property events, and
//! [`gone`] lets the window go.
//!
//! | Control | Control type | Patterns |
//! | --- | --- | --- |
//! | Button | Button | Invoke |
//! | Switch | CheckBox, "toggle switch" | Toggle |
//! | Checkbox, day pill | CheckBox | Toggle |
//! | Segmented toggle | Group of RadioButton | Selection; SelectionItem |
//! | Selectable cards | Group of RadioButton | Selection; SelectionItem |
//! | Stepper, time stepper | Spinner | RangeValue, Value |
//! | Drop-down | ComboBox, its menu a List of ListItem | ExpandCollapse, Selection, Value; SelectionItem |
//! | Card, card row | Group | |
//! | Text, note | Text; a note is the described-by of its control | |
//!
//! UI Automation calls a window's providers on the window's thread, as a window message would
//! run, so the providers read the form the way the window procedure does.

// The UI Automation constants keep their Windows names.
#![allow(non_upper_case_globals)]

mod tree;

#[cfg(test)]
mod tests;

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::mem::ManuallyDrop;

use windows::Win32::Foundation::{
    E_INVALIDARG, HWND, LPARAM, LRESULT, RECT, VARIANT_FALSE, VARIANT_TRUE, WPARAM,
};
use windows::Win32::System::Com::SAFEARRAY;
use windows::Win32::System::Ole::{SafeArrayCreateVector, SafeArrayPutElement};
use windows::Win32::System::Variant::{
    VARENUM, VARIANT, VARIANT_0_0_0, VT_ARRAY, VT_BOOL, VT_BSTR, VT_I4, VT_R8, VT_UNKNOWN,
    VariantClear,
};
use windows::Win32::UI::Accessibility::*;
use windows::Win32::UI::Input::KeyboardAndMouse::GetFocus;
use windows::Win32::UI::WindowsAndMessaging::{GetWindowRect, KillTimer, SetTimer};
use windows::core::{BOOL, BSTR, Error, HRESULT, IUnknown, Interface, PCWSTR, Result, implement};

pub use tree::{Action, Area, Change, Key, Nav, Node, Role, Tree, changes};

/// What the providers need from the window that hosts the form. The kit window implements it;
/// the tests implement it over a bare [`Form`](super::form::Form).
pub trait Source: Sync {
    /// The form's tree now, or `None` when the window is gone or busy, or when the call comes
    /// from another thread.
    fn tree(&self, hwnd: HWND) -> Option<Tree>;

    /// Does `action` as the user would ([`perform`]): repaints and hands the events to the
    /// window's caller. False when it cannot be done.
    fn act(&self, hwnd: HWND, action: Action) -> bool;

    /// A rectangle in the form's pixels ([`Area::Form`]) on screen, scrolling included.
    fn to_screen(&self, hwnd: HWND, r: RECT) -> RECT;

    /// The part of the screen where the form shows, to tell what is scrolled out of view.
    fn view(&self, hwnd: HWND) -> RECT;

    /// Whether the window has the keyboard focus.
    fn has_focus(&self, hwnd: HWND) -> bool {
        // SAFETY: no pointers.
        unsafe { GetFocus() == hwnd }
    }
}

fn hresult(code: u32) -> Error {
    Error::from_hresult(HRESULT(code as i32))
}

/// The element is gone: the window closed, or the element no longer exists.
fn not_available() -> Error {
    hresult(UIA_E_ELEMENTNOTAVAILABLE)
}

fn not_enabled() -> Error {
    hresult(UIA_E_ELEMENTNOTENABLED)
}

fn invalid_operation() -> Error {
    hresult(UIA_E_INVALIDOPERATION)
}

/// "No element": a null result with success, as UI Automation expects.
fn none<T>() -> Result<T> {
    Err(Error::empty())
}

// ---- Window entry points ----

thread_local! {
    /// The tree each window last told UI Automation about, to find what changed.
    static LAST: RefCell<HashMap<isize, Tree>> = RefCell::new(HashMap::new());
    /// Buttons to press once the client's Invoke has returned.
    static PRESSES: RefCell<VecDeque<(isize, &'static dyn Source, Key)>> =
        const { RefCell::new(VecDeque::new()) };
}

/// Answers `WM_GETOBJECT`: the provider tree for UI Automation's root object id, `None` for
/// anything else, which goes to `DefWindowProcW`.
pub fn get_object(
    hwnd: HWND,
    wparam: WPARAM,
    lparam: LPARAM,
    source: &'static dyn Source,
) -> Option<LRESULT> {
    if lparam.0 as i32 != UiaRootObjectId {
        return None;
    }
    if let Some(tree) = source.tree(hwnd) {
        LAST.with_borrow_mut(|last| last.insert(hwnd.0 as isize, tree));
    }
    let root: IRawElementProviderSimple = Element::new(hwnd, source, Key::Root).into_interface();
    // SAFETY: the provider lives as long as UI Automation holds it.
    Some(unsafe { UiaReturnRawElementProvider(hwnd, wparam, lparam, &root) })
}

/// Tells listening screen readers what changed in the form since the last call: names,
/// toggles, values, selection, the menu, then the focus. Call it after input and after the
/// caller changes the form. Cheap when no client listens.
pub fn changed(hwnd: HWND, source: &'static dyn Source) {
    // SAFETY: no pointers.
    if !unsafe { UiaClientsAreListening() }.as_bool() {
        LAST.with_borrow_mut(|last| last.remove(&(hwnd.0 as isize)));
        return;
    }
    let Some(now) = source.tree(hwnd) else {
        return;
    };
    let Some(before) = LAST.with_borrow_mut(|last| last.insert(hwnd.0 as isize, now.clone()))
    else {
        return;
    };
    let focused = source.has_focus(hwnd);
    for change in changes(&before, &now) {
        raise(hwnd, source, &change, focused);
    }
}

/// The window is going: UI Automation lets go of its providers.
pub fn gone(hwnd: HWND) {
    LAST.with_borrow_mut(|last| last.remove(&(hwnd.0 as isize)));
    PRESSES.with_borrow_mut(|p| p.retain(|&(h, ..)| h != hwnd.0 as isize));
    // SAFETY: no pointers; a null provider releases the window's.
    unsafe {
        UiaReturnRawElementProvider(
            hwnd,
            WPARAM(0),
            LPARAM(0),
            None::<&IRawElementProviderSimple>,
        );
    }
}

/// Raises the event for one change. A focus change is raised only while the window has the
/// focus: when it gets it, UI Automation asks the root for the focused element itself.
fn raise(hwnd: HWND, source: &'static dyn Source, change: &Change, focused: bool) {
    let simple =
        |key| -> IRawElementProviderSimple { Element::new(hwnd, source, key).into_interface() };
    let property = |key, id, old: Var, new: Var| {
        // SAFETY: both values live through the call.
        let _ = unsafe { UiaRaiseAutomationPropertyChangedEvent(&simple(key), id, &old.0, &new.0) };
    };
    match change {
        Change::Name(k, a, b) => property(*k, UIA_NamePropertyId, Var::str(a), Var::str(b)),
        Change::Toggle(k, a, b) => property(
            *k,
            UIA_ToggleToggleStatePropertyId,
            Var::i32(toggle_state(*a).0),
            Var::i32(toggle_state(*b).0),
        ),
        Change::Range(k, a, b) => property(
            *k,
            UIA_RangeValueValuePropertyId,
            Var::f64(*a),
            Var::f64(*b),
        ),
        Change::Value(k, a, b) => property(*k, UIA_ValueValuePropertyId, Var::str(a), Var::str(b)),
        Change::Expanded(k, a, b) => property(
            *k,
            UIA_ExpandCollapseExpandCollapseStatePropertyId,
            Var::i32(expand_state(*a).0),
            Var::i32(expand_state(*b).0),
        ),
        Change::Selected(k, a, b) => {
            property(
                *k,
                UIA_SelectionItemIsSelectedPropertyId,
                Var::bool(*a),
                Var::bool(*b),
            );
            if *b {
                // SAFETY: the provider lives through the call.
                let _ = unsafe {
                    UiaRaiseAutomationEvent(&simple(*k), UIA_SelectionItem_ElementSelectedEventId)
                };
            }
        }
        Change::Focus(k) if focused => {
            // SAFETY: as above.
            let _ =
                unsafe { UiaRaiseAutomationEvent(&simple(*k), UIA_AutomationFocusChangedEventId) };
        }
        Change::Focus(_) => {}
    }
}

fn toggle_state(on: bool) -> ToggleState {
    if on { ToggleState_On } else { ToggleState_Off }
}

fn expand_state(open: bool) -> ExpandCollapseState {
    if open {
        ExpandCollapseState_Expanded
    } else {
        ExpandCollapseState_Collapsed
    }
}

/// Presses the queued buttons. Invoke must return before the press runs, since a press may
/// show a message box, which would hold the client until it closes.
unsafe extern "system" fn press_queued(_: HWND, _: u32, id: usize, _: u32) {
    // SAFETY: a thread timer with no window.
    let _ = unsafe { KillTimer(None, id) };
    while let Some((hwnd, source, key)) = PRESSES.with_borrow_mut(VecDeque::pop_front) {
        source.act(HWND(hwnd as _), Action::Invoke(key));
    }
}

// ---- Values ----

/// A VARIANT this side owns and clears.
struct Var(VARIANT);

impl Var {
    fn new(vt: VARENUM, fill: impl FnOnce(&mut VARIANT_0_0_0)) -> Var {
        let mut v = VARIANT::default();
        // SAFETY: a zeroed VARIANT gets its type tag and the matching field.
        unsafe {
            let inner = &mut *v.Anonymous.Anonymous;
            inner.vt = vt;
            fill(&mut inner.Anonymous);
        }
        Var(v)
    }

    fn str(s: &str) -> Var {
        Var::new(VT_BSTR, |u| u.bstrVal = ManuallyDrop::new(BSTR::from(s)))
    }

    fn i32(n: i32) -> Var {
        Var::new(VT_I4, |u| u.lVal = n)
    }

    fn f64(n: f64) -> Var {
        Var::new(VT_R8, |u| u.dblVal = n)
    }

    fn bool(b: bool) -> Var {
        Var::new(VT_BOOL, |u| {
            u.boolVal = if b { VARIANT_TRUE } else { VARIANT_FALSE }
        })
    }

    fn unknown(p: IUnknown) -> Var {
        Var::new(VT_UNKNOWN, |u| u.punkVal = ManuallyDrop::new(Some(p)))
    }

    fn unknowns(all: &[IUnknown]) -> Var {
        let array = unknown_array(all);
        Var::new(VT_ARRAY | VT_UNKNOWN, |u| u.parray = array)
    }

    /// Hands the value over to the caller, who clears it.
    fn give(self) -> VARIANT {
        let me = ManuallyDrop::new(self);
        // SAFETY: `me` is never dropped, so the value moves out once.
        unsafe { std::ptr::read(&me.0) }
    }
}

impl Drop for Var {
    fn drop(&mut self) {
        // SAFETY: the VARIANT was built by `Var::new` and is cleared once.
        let _ = unsafe { VariantClear(&mut self.0) };
    }
}

fn int_array(values: &[i32]) -> *mut SAFEARRAY {
    // SAFETY: a vector of `values.len()` four-byte integers, each put at its index.
    unsafe {
        let array = SafeArrayCreateVector(VT_I4, 0, values.len() as u32);
        if !array.is_null() {
            for (at, v) in (0..).zip(values) {
                let _ = SafeArrayPutElement(array, &at, v as *const i32 as _);
            }
        }
        array
    }
}

fn unknown_array(all: &[IUnknown]) -> *mut SAFEARRAY {
    // SAFETY: as above; the array takes its own reference to each interface.
    unsafe {
        let array = SafeArrayCreateVector(VT_UNKNOWN, 0, all.len() as u32);
        if !array.is_null() {
            for (at, p) in (0..).zip(all) {
                let _ = SafeArrayPutElement(array, &at, p.as_raw());
            }
        }
        array
    }
}

// ---- The provider ----

/// One element of a kit window's tree, as UI Automation sees it. It holds no state of its own:
/// each call reads the form afresh, so a provider a client keeps stays right, and one whose
/// element has gone says so.
#[implement(
    IRawElementProviderSimple,
    IRawElementProviderFragment,
    IRawElementProviderFragmentRoot,
    IInvokeProvider,
    IToggleProvider,
    ISelectionProvider,
    ISelectionItemProvider,
    IRangeValueProvider,
    IValueProvider,
    IExpandCollapseProvider
)]
struct Element {
    /// The window, as a number, so the provider can move between threads as COM requires.
    hwnd: isize,
    source: &'static dyn Source,
    key: Key,
}

impl Element {
    fn new(hwnd: HWND, source: &'static dyn Source, key: Key) -> windows::core::ComObject<Element> {
        windows::core::ComObject::new(Element {
            hwnd: hwnd.0 as isize,
            source,
            key,
        })
    }

    fn hwnd(&self) -> HWND {
        HWND(self.hwnd as _)
    }

    fn other(&self, key: Key) -> windows::core::ComObject<Element> {
        Element::new(self.hwnd(), self.source, key)
    }

    fn tree(&self) -> Result<Tree> {
        self.source.tree(self.hwnd()).ok_or_else(not_available)
    }

    /// Runs `f` on this element's node in a fresh tree.
    fn with<R>(&self, f: impl FnOnce(&Tree, &Node) -> Result<R>) -> Result<R> {
        let tree = self.tree()?;
        let node = tree.get(self.key).ok_or_else(not_available)?;
        f(&tree, node)
    }

    fn act(&self, action: Action, refused: fn() -> Error) -> Result<()> {
        self.with(|_, _| Ok(()))?;
        if self.source.act(self.hwnd(), action) {
            Ok(())
        } else {
            Err(refused())
        }
    }

    fn screen(&self, area: Area) -> Option<RECT> {
        match area {
            Area::None => None,
            Area::Form(r) => Some(self.source.to_screen(self.hwnd(), r)),
            Area::Screen(r) => Some(r),
        }
    }

    fn offscreen(&self, area: Area) -> bool {
        let view = self.source.view(self.hwnd());
        match area {
            Area::None => true,
            Area::Screen(_) => false,
            Area::Form(_) => self.screen(area).is_none_or(|r| {
                r.right <= view.left
                    || r.left >= view.right
                    || r.bottom <= view.top
                    || r.top >= view.bottom
            }),
        }
    }

    /// The node's control type, and its own words for it, if any.
    fn control_type(role: Role) -> (UIA_CONTROLTYPE_ID, Option<&'static str>) {
        match role {
            Role::Root => (UIA_WindowControlTypeId, None),
            Role::Group | Role::RadioGroup => (UIA_GroupControlTypeId, None),
            Role::Text => (UIA_TextControlTypeId, None),
            Role::Button => (UIA_ButtonControlTypeId, None),
            Role::CheckBox => (UIA_CheckBoxControlTypeId, None),
            Role::Switch => (UIA_CheckBoxControlTypeId, Some("toggle switch")),
            Role::Radio => (UIA_RadioButtonControlTypeId, None),
            Role::Spinner => (UIA_SpinnerControlTypeId, None),
            Role::ComboBox => (UIA_ComboBoxControlTypeId, None),
            Role::List => (UIA_ListControlTypeId, None),
            Role::ListItem => (UIA_ListItemControlTypeId, None),
        }
    }

    /// Whether a node of `role` offers `pattern`.
    fn offers(role: Role, pattern: UIA_PATTERN_ID) -> bool {
        match role {
            Role::Button => pattern == UIA_InvokePatternId,
            Role::CheckBox | Role::Switch => pattern == UIA_TogglePatternId,
            Role::RadioGroup | Role::List => pattern == UIA_SelectionPatternId,
            Role::Radio | Role::ListItem => pattern == UIA_SelectionItemPatternId,
            Role::Spinner => pattern == UIA_RangeValuePatternId || pattern == UIA_ValuePatternId,
            Role::ComboBox => [
                UIA_ExpandCollapsePatternId,
                UIA_SelectionPatternId,
                UIA_ValuePatternId,
            ]
            .contains(&pattern),
            Role::Root | Role::Group | Role::Text => false,
        }
    }

    fn property(&self, tree: &Tree, node: &Node, id: UIA_PROPERTY_ID) -> Option<Var> {
        let unknown = |key| -> IUnknown { self.other(key).into_interface() };
        let (control_type, words) = Element::control_type(node.role);
        if node.role == Role::Root {
            // The window's frame provider answers the rest: name, place, focus.
            return match id {
                UIA_ControlTypePropertyId => Some(Var::i32(control_type.0)),
                _ => None,
            };
        }
        Some(match id {
            UIA_ControlTypePropertyId => Var::i32(control_type.0),
            UIA_LocalizedControlTypePropertyId => Var::str(words?),
            UIA_NamePropertyId => Var::str(&node.name),
            UIA_AutomationIdPropertyId => Var::str(&match node.key {
                Key::Option(_, k) | Key::Entry(_, k) => format!("{}.{k}", node.id?),
                Key::Menu(_) => format!("{}.menu", node.id?),
                _ => node.id?.to_string(),
            }),
            UIA_IsKeyboardFocusablePropertyId => Var::bool(node.role.focusable()),
            UIA_HasKeyboardFocusPropertyId => {
                Var::bool(tree.focus() == Some(node.key) && self.source.has_focus(self.hwnd()))
            }
            UIA_IsEnabledPropertyId => Var::bool(true),
            UIA_IsOffscreenPropertyId => Var::bool(self.offscreen(node.area)),
            UIA_LabeledByPropertyId => Var::unknown(unknown(node.labeled_by?)),
            UIA_DescribedByPropertyId => Var::unknowns(&[unknown(node.described_by?)]),
            UIA_HelpTextPropertyId if !node.help.is_empty() => Var::str(&node.help),
            UIA_HeadingLevelPropertyId if node.heading > 0 => Var::i32(match node.heading {
                1 => HeadingLevel1.0,
                _ => HeadingLevel2.0,
            }),
            UIA_PositionInSetPropertyId => Var::i32(node.set?.0 as i32),
            UIA_SizeOfSetPropertyId => Var::i32(node.set?.1 as i32),
            _ => return None,
        })
    }

    /// The providers of `keys`, as an array for UI Automation.
    fn array(&self, keys: &[Key]) -> *mut SAFEARRAY {
        let all: Vec<IUnknown> = keys
            .iter()
            .map(|&k| self.other(k).into_interface())
            .collect();
        unknown_array(&all)
    }
}

impl IRawElementProviderSimple_Impl for Element_Impl {
    fn ProviderOptions(&self) -> Result<ProviderOptions> {
        Ok(ProviderOptions_ServerSideProvider)
    }

    fn GetPatternProvider(&self, pattern: UIA_PATTERN_ID) -> Result<IUnknown> {
        let role = self.with(|_, node| Ok(node.role))?;
        if Element::offers(role, pattern) {
            Ok(self.other(self.key).into_interface())
        } else {
            none()
        }
    }

    fn GetPropertyValue(&self, id: UIA_PROPERTY_ID) -> Result<VARIANT> {
        self.with(|tree, node| {
            Ok(self
                .property(tree, node, id)
                .map_or_else(VARIANT::default, Var::give))
        })
    }

    fn HostRawElementProvider(&self) -> Result<IRawElementProviderSimple> {
        if self.key == Key::Root {
            // SAFETY: no pointers.
            unsafe { UiaHostProviderFromHwnd(self.hwnd()) }
        } else {
            none()
        }
    }
}

impl IRawElementProviderFragment_Impl for Element_Impl {
    fn Navigate(&self, direction: NavigateDirection) -> Result<IRawElementProviderFragment> {
        let nav = match direction {
            NavigateDirection_Parent => Nav::Parent,
            NavigateDirection_NextSibling => Nav::Next,
            NavigateDirection_PreviousSibling => Nav::Previous,
            NavigateDirection_FirstChild => Nav::First,
            NavigateDirection_LastChild => Nav::Last,
            _ => return none(),
        };
        match self.tree()?.navigate(self.key, nav) {
            Some(key) => Ok(self.other(key).into_interface()),
            None => none(),
        }
    }

    fn GetRuntimeId(&self) -> Result<*mut SAFEARRAY> {
        if self.key == Key::Root {
            // The window's own runtime id serves the root.
            return Ok(std::ptr::null_mut());
        }
        let mut id = vec![UiaAppendRuntimeId as i32];
        id.extend(self.key.runtime_id());
        Ok(int_array(&id))
    }

    fn BoundingRectangle(&self) -> Result<UiaRect> {
        if self.key == Key::Root {
            let mut r = RECT::default();
            // SAFETY: `r` is a valid out-pointer; a stale window leaves it empty.
            let _ = unsafe { GetWindowRect(self.hwnd(), &mut r) };
            return Ok(uia_rect(r));
        }
        let area = self.with(|_, node| Ok(node.area))?;
        Ok(self.screen(area).map(uia_rect).unwrap_or_default())
    }

    fn GetEmbeddedFragmentRoots(&self) -> Result<*mut SAFEARRAY> {
        Ok(std::ptr::null_mut())
    }

    fn SetFocus(&self) -> Result<()> {
        self.act(Action::Focus(self.key), not_enabled)
    }

    fn FragmentRoot(&self) -> Result<IRawElementProviderFragmentRoot> {
        Ok(self.other(Key::Root).into_interface())
    }
}

fn uia_rect(r: RECT) -> UiaRect {
    UiaRect {
        left: f64::from(r.left),
        top: f64::from(r.top),
        width: f64::from(r.right - r.left),
        height: f64::from(r.bottom - r.top),
    }
}

impl IRawElementProviderFragmentRoot_Impl for Element_Impl {
    fn ElementProviderFromPoint(&self, x: f64, y: f64) -> Result<IRawElementProviderFragment> {
        let tree = self.tree()?;
        let (x, y) = (x as i32, y as i32);
        let hit = tree
            .nodes()
            .iter()
            .filter(|n| n.role != Role::Root && !self.offscreen(n.area))
            .filter(|n| {
                self.screen(n.area)
                    .is_some_and(|r| x >= r.left && x < r.right && y >= r.top && y < r.bottom)
            })
            .max_by_key(|n| tree.depth(n.key))
            .map_or(Key::Root, |n| n.key);
        Ok(self.other(hit).into_interface())
    }

    fn GetFocus(&self) -> Result<IRawElementProviderFragment> {
        match self.tree()?.focus() {
            Some(key) => Ok(self.other(key).into_interface()),
            None => none(),
        }
    }
}

impl IInvokeProvider_Impl for Element_Impl {
    fn Invoke(&self) -> Result<()> {
        let role = self.with(|_, node| Ok(node.role))?;
        if role != Role::Button {
            return Err(not_enabled());
        }
        // SAFETY: the provider lives through the call.
        let _ = unsafe {
            UiaRaiseAutomationEvent(
                &self
                    .other(self.key)
                    .into_interface::<IRawElementProviderSimple>(),
                UIA_Invoke_InvokedEventId,
            )
        };
        PRESSES.with_borrow_mut(|p| p.push_back((self.hwnd, self.source, self.key)));
        // SAFETY: a thread timer with no window; `press_queued` kills it.
        unsafe { SetTimer(None, 0, 0, Some(press_queued)) };
        Ok(())
    }
}

impl IToggleProvider_Impl for Element_Impl {
    fn Toggle(&self) -> Result<()> {
        self.act(Action::Toggle(self.key), not_enabled)
    }

    fn ToggleState(&self) -> Result<ToggleState> {
        self.with(|_, node| node.toggle.map(toggle_state).ok_or_else(invalid_operation))
    }
}

impl ISelectionProvider_Impl for Element_Impl {
    fn GetSelection(&self) -> Result<*mut SAFEARRAY> {
        let selected = self.tree()?.selection(self.key);
        Ok(self.array(&selected))
    }

    fn CanSelectMultiple(&self) -> Result<BOOL> {
        Ok(false.into())
    }

    fn IsSelectionRequired(&self) -> Result<BOOL> {
        // A segmented toggle always has a segment; a drop-down may show Custom, which no entry is.
        self.with(|_, node| Ok((node.role == Role::RadioGroup).into()))
    }
}

impl ISelectionItemProvider_Impl for Element_Impl {
    fn Select(&self) -> Result<()> {
        self.act(Action::Select(self.key), not_enabled)
    }

    fn AddToSelection(&self) -> Result<()> {
        // One selection only: adding is selecting, and only the selected one may be "added".
        match self.with(|_, node| Ok(node.selected))? {
            Some(true) => Ok(()),
            _ => Err(invalid_operation()),
        }
    }

    fn RemoveFromSelection(&self) -> Result<()> {
        match self.with(|_, node| Ok(node.selected))? {
            Some(false) => Ok(()),
            _ => Err(invalid_operation()),
        }
    }

    fn IsSelected(&self) -> Result<BOOL> {
        self.with(|_, node| Ok((node.selected == Some(true)).into()))
    }

    fn SelectionContainer(&self) -> Result<IRawElementProviderSimple> {
        let parent = self.with(|_, node| node.parent.ok_or_else(not_available))?;
        Ok(self.other(parent).into_interface())
    }
}

impl Element_Impl {
    fn range(&self) -> Result<tree::Range> {
        self.with(|_, node| node.range.ok_or_else(invalid_operation))
    }
}

impl IRangeValueProvider_Impl for Element_Impl {
    fn SetValue(&self, value: f64) -> Result<()> {
        let range = self.range()?;
        if !(range.min..=range.max).contains(&value) {
            return Err(E_INVALIDARG.into());
        }
        self.act(Action::SetNumber(self.key, value.round() as i32), || {
            E_INVALIDARG.into()
        })
    }

    fn Value(&self) -> Result<f64> {
        Ok(self.range()?.value)
    }

    fn IsReadOnly(&self) -> Result<BOOL> {
        Ok(false.into())
    }

    fn Maximum(&self) -> Result<f64> {
        Ok(self.range()?.max)
    }

    fn Minimum(&self) -> Result<f64> {
        Ok(self.range()?.min)
    }

    fn LargeChange(&self) -> Result<f64> {
        Ok(self.range()?.large)
    }

    fn SmallChange(&self) -> Result<f64> {
        Ok(self.range()?.small)
    }
}

impl IValueProvider_Impl for Element_Impl {
    fn SetValue(&self, value: &PCWSTR) -> Result<()> {
        if self.with(|_, node| Ok(node.role))? != Role::Spinner {
            return Err(not_enabled());
        }
        // SAFETY: UI Automation passes a valid string, or null.
        let text = if value.is_null() {
            String::new()
        } else {
            unsafe { value.to_string() }.map_err(|_| Error::from(E_INVALIDARG))?
        };
        self.act(Action::Type(self.key, text), || E_INVALIDARG.into())
    }

    fn Value(&self) -> Result<BSTR> {
        self.with(|_, node| Ok(BSTR::from(node.value.as_deref().unwrap_or_default())))
    }

    fn IsReadOnly(&self) -> Result<BOOL> {
        self.with(|_, node| Ok((node.role != Role::Spinner).into()))
    }
}

impl IExpandCollapseProvider_Impl for Element_Impl {
    fn Expand(&self) -> Result<()> {
        self.act(Action::Expand(self.key), not_enabled)
    }

    fn Collapse(&self) -> Result<()> {
        self.act(Action::Collapse(self.key), not_enabled)
    }

    fn ExpandCollapseState(&self) -> Result<ExpandCollapseState> {
        self.with(|_, node| {
            node.expanded
                .map(expand_state)
                .ok_or_else(invalid_operation)
        })
    }
}
