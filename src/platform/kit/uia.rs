//! UI Automation for kit windows (issue #80): painted controls have no window of their own, so
//! the kit tells screen readers what they are.
//!
//! The window answers `WM_GETOBJECT` with [`get_object`], which hands UI Automation the root of
//! a provider tree: the window as the fragment root, one fragment per element of [`Tree`].
//! [`changed`], called after anything that may change the form, raises the focus and property
//! events, and [`gone`] lets the window go.
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
//! Threads: UI Automation calls the providers on the window's thread, and some calls on a thread
//! of its own. The form lives on the window's thread, so a provider reads a fresh [`Snapshot`]
//! there, and the last one the window's thread took anywhere else. What a client asks to be done
//! runs on the window's thread: at once when asked there, else sent to it. An Invoke is always
//! posted, so a Save that shows a message box does not hold the screen reader.

// The UI Automation constants keep their Windows names.
#![allow(non_upper_case_globals)]

mod tree;

#[cfg(test)]
mod tests;

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::mem::ManuallyDrop;
use std::sync::{Arc, Mutex, MutexGuard};

use windows::Win32::Foundation::{
    E_INVALIDARG, HWND, LPARAM, LRESULT, RECT, VARIANT_FALSE, VARIANT_TRUE, WPARAM,
};
use windows::Win32::System::Com::SAFEARRAY;
use windows::Win32::System::Ole::{SafeArrayCreateVector, SafeArrayPutElement};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::System::Variant::{
    VARENUM, VARIANT, VARIANT_0_0_0, VT_ARRAY, VT_BOOL, VT_BSTR, VT_I4, VT_R8, VT_UNKNOWN,
    VariantClear,
};
use windows::Win32::UI::Accessibility::*;
use windows::Win32::UI::Input::KeyboardAndMouse::GetFocus;
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowRect, GetWindowThreadProcessId, PostMessageW, SMTO_NORMAL, SendMessageTimeoutW,
    WM_GETOBJECT,
};
use windows::core::{BOOL, BSTR, Error, HRESULT, IUnknown, Interface, PCWSTR, Result, implement};

use super::form::{parse_number, parse_time};
pub use tree::{Action, Area, Change, Key, Nav, Node, Role, Tree, changes, perform};

/// What the providers need from the window that hosts the form. The kit window implements it;
/// the tests implement it over a bare [`Form`](super::form::Form).
///
/// [`Source::on_thread`] and [`Source::wake`] are called from any thread; the rest only on the
/// window's thread.
pub trait Source: Sync {
    /// The form's tree now, or `None` when the window is gone or busy.
    fn tree(&self, hwnd: HWND) -> Option<Tree>;

    /// Does `action` as the user would ([`perform`]): repaints and hands the events to the
    /// window's caller. False when it cannot be done.
    fn act(&self, hwnd: HWND, action: Action) -> bool;

    /// The part of a rectangle in the form's pixels ([`Area::Form`]) that shows, on screen:
    /// scrolled, and cut where the scrolling content goes under the caption or the footer
    /// band. `None` when none of it shows.
    fn on_screen(&self, hwnd: HWND, r: RECT) -> Option<RECT>;

    /// Whether the window has the keyboard focus.
    fn has_focus(&self, hwnd: HWND) -> bool {
        // SAFETY: no pointers.
        unsafe { GetFocus() == hwnd }
    }

    /// Whether this is the window's thread, where the form may be read.
    fn on_thread(&self, hwnd: HWND) -> bool {
        window_thread(hwnd)
    }

    /// Asks the window's thread to call [`run_posted`]; with `wait`, waits until it has, for
    /// a moment at most.
    fn wake(&self, hwnd: HWND, wait: bool) {
        wake_window(hwnd, wait);
    }
}

/// Whether the calling thread is `hwnd`'s.
pub fn window_thread(hwnd: HWND) -> bool {
    // SAFETY: no pointers; a stale window gives thread 0, which is no thread.
    unsafe { GetWindowThreadProcessId(hwnd, None) == GetCurrentThreadId() }
}

/// The object id that asks [`get_object`] to run the posted actions: "bpua". Object ids an app
/// makes up are positive.
const RUN_POSTED: i32 = 0x6270_7561;

/// How long a client's call waits for the window's thread to do what it asked. The action
/// stays queued if the wait runs out, so it still happens.
const WAIT_MS: u32 = 1000;

/// Sends or posts the window the message that runs [`run_posted`].
pub fn wake_window(hwnd: HWND, wait: bool) {
    let lparam = LPARAM(RUN_POSTED as isize);
    // SAFETY: no pointers; a stale window fails harmlessly.
    unsafe {
        if wait {
            SendMessageTimeoutW(
                hwnd,
                WM_GETOBJECT,
                WPARAM(0),
                lparam,
                SMTO_NORMAL,
                WAIT_MS,
                None,
            );
        } else {
            let _ = PostMessageW(Some(hwnd), WM_GETOBJECT, WPARAM(0), lparam);
        }
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

// ---- Snapshots ----

/// The tree, with what else a provider reads that needs the window's thread: the part of each
/// element that shows on screen, and whether the window has the focus.
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub tree: Tree,
    /// Only the elements that show.
    screen: HashMap<Key, RECT>,
    focused: bool,
}

impl Snapshot {
    /// Takes one on the window's thread.
    fn take(hwnd: HWND, source: &dyn Source) -> Option<Snapshot> {
        let tree = source.tree(hwnd)?;
        let screen = tree
            .nodes()
            .iter()
            .filter_map(|n| match n.area {
                Area::None => None,
                Area::Form(r) => Some((n.key, source.on_screen(hwnd, r)?)),
                // The open menu and its entries.
                Area::Screen(r) => Some((n.key, r)),
            })
            .collect();
        Some(Snapshot {
            tree,
            screen,
            focused: source.has_focus(hwnd),
        })
    }

    /// The part of the element that shows, on screen.
    fn rect(&self, key: Key) -> Option<RECT> {
        self.screen.get(&key).copied()
    }

    /// Off screen: nowhere, such as a closed menu, or scrolled wholly out of view.
    fn offscreen(&self, key: Key) -> bool {
        !self.screen.contains_key(&key)
    }
}

/// The last snapshot of each window with providers out, for calls from other threads.
static SNAPSHOTS: Mutex<Vec<(isize, Arc<Snapshot>)>> = Mutex::new(Vec::new());

/// An action waiting for its window's thread.
type Posted = (isize, &'static dyn Source, Action);
static POSTED: Mutex<VecDeque<Posted>> = Mutex::new(VecDeque::new());

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn keep(hwnd: HWND, snapshot: Arc<Snapshot>) {
    let mut all = lock(&SNAPSHOTS);
    match all.iter_mut().find(|(h, _)| *h == hwnd.0 as isize) {
        Some(slot) => slot.1 = snapshot,
        None => all.push((hwnd.0 as isize, snapshot)),
    }
}

fn kept(hwnd: HWND) -> Option<Arc<Snapshot>> {
    lock(&SNAPSHOTS)
        .iter()
        .find(|(h, _)| *h == hwnd.0 as isize)
        .map(|(_, s)| s.clone())
}

/// The window's snapshot for a provider: fresh on its thread, else the last one kept.
fn snapshot(hwnd: HWND, source: &dyn Source) -> Option<Arc<Snapshot>> {
    if source.on_thread(hwnd) {
        let fresh = Arc::new(Snapshot::take(hwnd, source)?);
        keep(hwnd, fresh.clone());
        Some(fresh)
    } else {
        kept(hwnd)
    }
}

/// Queues `action` for the window's thread; with `wait`, waits for it a moment.
fn post(hwnd: HWND, source: &'static dyn Source, action: Action, wait: bool) {
    lock(&POSTED).push_back((hwnd.0 as isize, source, action));
    source.wake(hwnd, wait);
}

/// Runs the actions posted for `hwnd`, in order, then tells clients what changed. Called on the
/// window's thread, by [`get_object`] or a test's own wake-up.
pub fn run_posted(hwnd: HWND) {
    let mut last = None;
    loop {
        let next = {
            let mut posted = lock(&POSTED);
            let at = posted.iter().position(|(h, ..)| *h == hwnd.0 as isize);
            at.and_then(|at| posted.remove(at))
        };
        let Some((_, source, action)) = next else {
            break;
        };
        source.act(hwnd, action);
        last = Some(source);
    }
    if let Some(source) = last {
        changed(hwnd, source);
    }
}

// ---- Window entry points ----

thread_local! {
    /// The tree each window last told listening clients about, to find what changed.
    static TOLD: RefCell<HashMap<isize, Tree>> = RefCell::new(HashMap::new());
}

/// Answers `WM_GETOBJECT`: the provider tree for UI Automation's root object id, the posted
/// actions for [`RUN_POSTED`], `None` for anything else, which goes to `DefWindowProcW`.
pub fn get_object(
    hwnd: HWND,
    wparam: WPARAM,
    lparam: LPARAM,
    source: &'static dyn Source,
) -> Option<LRESULT> {
    match lparam.0 as i32 {
        UiaRootObjectId => {
            if let Some(now) = Snapshot::take(hwnd, source) {
                TOLD.with_borrow_mut(|told| told.insert(hwnd.0 as isize, now.tree.clone()));
                keep(hwnd, Arc::new(now));
            }
            let root: IRawElementProviderSimple =
                Element::new(hwnd, source, Key::Root).into_interface();
            // SAFETY: the provider lives as long as UI Automation holds it.
            Some(unsafe { UiaReturnRawElementProvider(hwnd, wparam, lparam, &root) })
        }
        RUN_POSTED => {
            run_posted(hwnd);
            Some(LRESULT(0))
        }
        _ => None,
    }
}

/// After anything that may change the form: keeps the snapshot for other threads fresh, and
/// tells listening clients what changed since the last call: names, toggles, values,
/// selection, the menu, then the focus. Costs nothing until a client asks for the window.
pub fn changed(hwnd: HWND, source: &'static dyn Source) {
    if kept(hwnd).is_none() {
        return;
    }
    let Some(now) = Snapshot::take(hwnd, source) else {
        return;
    };
    let focused = now.focused;
    let tree = now.tree.clone();
    keep(hwnd, Arc::new(now));
    // SAFETY: no pointers.
    if !unsafe { UiaClientsAreListening() }.as_bool() {
        TOLD.with_borrow_mut(|told| told.remove(&(hwnd.0 as isize)));
        return;
    }
    let Some(before) = TOLD.with_borrow_mut(|told| told.insert(hwnd.0 as isize, tree.clone()))
    else {
        return;
    };
    for change in changes(&before, &tree) {
        raise(hwnd, source, &change, focused);
    }
}

/// The window is going: UI Automation lets go of its providers.
pub fn gone(hwnd: HWND) {
    let h = hwnd.0 as isize;
    TOLD.with_borrow_mut(|told| told.remove(&h));
    lock(&SNAPSHOTS).retain(|(w, _)| *w != h);
    lock(&POSTED).retain(|(w, ..)| *w != h);
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

    fn snapshot(&self) -> Result<Arc<Snapshot>> {
        snapshot(self.hwnd(), self.source).ok_or_else(not_available)
    }

    /// Runs `f` on this element's node in the window's snapshot.
    fn with<R>(&self, f: impl FnOnce(&Snapshot, &Node) -> Result<R>) -> Result<R> {
        let snap = self.snapshot()?;
        let node = snap.tree.get(self.key).ok_or_else(not_available)?;
        f(&snap, node)
    }

    /// Does `action` on the window's thread, so the client reads the result straight after:
    /// at once when called there, else sent there. Only a refusal on the window's own thread
    /// comes back as an error.
    fn act(&self, action: Action, refused: fn() -> Error) -> Result<()> {
        self.with(|_, _| Ok(()))?;
        if !self.source.on_thread(self.hwnd()) {
            post(self.hwnd(), self.source, action, true);
            Ok(())
        } else if self.source.act(self.hwnd(), action) {
            Ok(())
        } else {
            Err(refused())
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

    fn property(&self, snap: &Snapshot, node: &Node, id: UIA_PROPERTY_ID) -> Option<Var> {
        let unknown = |key| -> IUnknown { self.other(key).into_interface() };
        let (control_type, words) = Element::control_type(node.role);
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
                Var::bool(snap.tree.focus() == Some(node.key) && snap.focused)
            }
            UIA_IsEnabledPropertyId => Var::bool(true),
            UIA_IsOffscreenPropertyId => Var::bool(snap.offscreen(node.key)),
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
        if self.key == Key::Root {
            // UI Automation asks the root from a thread of its own as it takes the provider
            // over, so the root answers without the form. The window's frame provider answers
            // the rest: name, place, focus.
            return Ok(match id {
                UIA_ControlTypePropertyId => Var::i32(UIA_WindowControlTypeId.0).give(),
                _ => VARIANT::default(),
            });
        }
        self.with(|snap, node| {
            Ok(self
                .property(snap, node, id)
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
        match self.snapshot()?.tree.navigate(self.key, nav) {
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
        self.with(|snap, _| Ok(snap.rect(self.key).map(uia_rect).unwrap_or_default()))
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
        let snap = self.snapshot()?;
        let (x, y) = (x as i32, y as i32);
        let hit = snap
            .tree
            .nodes()
            .iter()
            .filter(|n| n.role != Role::Root && !snap.offscreen(n.key))
            .filter(|n| {
                snap.rect(n.key)
                    .is_some_and(|r| x >= r.left && x < r.right && y >= r.top && y < r.bottom)
            })
            .max_by_key(|n| snap.tree.depth(n.key))
            .map_or(Key::Root, |n| n.key);
        Ok(self.other(hit).into_interface())
    }

    fn GetFocus(&self) -> Result<IRawElementProviderFragment> {
        match self.snapshot()?.tree.focus() {
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
        // Always later: the press may show a message box, which must not hold the client.
        post(self.hwnd(), self.source, Action::Invoke(self.key), false);
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
        let selected = self.snapshot()?.tree.selection(self.key);
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
        // Checked here too, for a call that is posted and cannot fail later.
        let typed = text.trim();
        if parse_number(typed).is_none() && parse_time(typed).is_none() {
            return Err(E_INVALIDARG.into());
        }
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
