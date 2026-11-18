//! The settings form on a real kit window, through UI Automation: the hooks in
//! [`window`](crate::platform::kit::window) hand out the providers, the bounds follow the
//! scroll and are cut where the content goes under the footer band, and moving the focus to a
//! control scrolled out of view brings it into view before clients hear of it.
//!
//! The window runs on a thread of its own with a pretend work area 480 pixels tall, so it
//! scrolls the same on any screen.

use std::sync::mpsc;
use std::thread;

use windows::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetClientRect, GetMessageW, GetWindowThreadProcessId, MSG,
    PostThreadMessageW, TranslateMessage, WM_QUIT,
};

use super::super::kept;
use super::client::{Seen, one_client, read};
use super::*;
use crate::platform::kit::window::{self, Spec, pretend_work_area};

/// Opens the settings form on a hidden kit window on a thread of its own, pumping its messages
/// until told to quit; then closes it. Returns the window as a number.
fn open_settings() -> (isize, thread::JoinHandle<()>) {
    let (tx, rx) = mpsc::channel();
    let thread = thread::spawn(move || {
        pretend_work_area(Some(rect(0, 0, 1024, 480)));
        let spec = Spec {
            caption: "kit uia test".into(),
            width: settings_window::WIDTH,
            height: 0,
            items: settings_items(),
            at: Some(POINT { x: 0, y: 0 }),
            owner: None,
            frame: window::Frame::Dialog,
        };
        let handle = window::open(spec, |_, _| {}).unwrap();
        tx.send(handle.hwnd().0 as isize).unwrap();
        let mut msg = MSG::default();
        // SAFETY: a plain message loop on this thread.
        unsafe {
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        handle.close();
        pretend_work_area(None);
    });
    (rx.recv().unwrap(), thread)
}

fn quit(hwnd: HWND, thread: thread::JoinHandle<()>) {
    // SAFETY: no pointers.
    unsafe {
        let id = GetWindowThreadProcessId(hwnd, None);
        PostThreadMessageW(id, WM_QUIT, WPARAM(0), LPARAM(0)).unwrap();
    }
    thread.join().unwrap();
}

/// The window's client area on screen.
fn client(hwnd: HWND) -> RECT {
    let mut r = RECT::default();
    let mut corner = POINT::default();
    // SAFETY: valid out-pointers.
    unsafe {
        GetClientRect(hwnd, &mut r).unwrap();
        let _ = ClientToScreen(hwnd, &mut corner);
    }
    rect(corner.x, corner.y, r.right, r.bottom)
}

fn find(seen: &Seen, control: UIA_CONTROLTYPE_ID, name: &str) -> IUIAutomationElement {
    seen.iter()
        .find(|(c, n, _)| *c == control.0 && n == name)
        .map(|(.., e)| e.clone())
        .unwrap_or_else(|| panic!("no {name:?}"))
}

fn within(inner: &UiaRect, outer: &RECT) -> bool {
    inner.width > 0.0
        && inner.height > 0.0
        && inner.left >= f64::from(outer.left)
        && inner.top >= f64::from(outer.top)
        && inner.left + inner.width <= f64::from(outer.right)
        && inner.top + inner.height <= f64::from(outer.bottom)
}

fn bounds(e: &IUIAutomationElement) -> UiaRect {
    // SAFETY: a live element.
    let r = unsafe { e.CurrentBoundingRectangle() }.unwrap();
    UiaRect {
        left: f64::from(r.left),
        top: f64::from(r.top),
        width: f64::from(r.right - r.left),
        height: f64::from(r.bottom - r.top),
    }
}

#[test]
fn a_ui_automation_client_reads_and_drives_a_scrolled_kit_window() {
    let _one = one_client();
    let expected: Vec<(i32, String)> = tree(&settings())
        .nodes()
        .iter()
        .skip(1)
        .map(|n| (Element::control_type(n.role).0.0, n.name.clone()))
        .collect();
    let (hwnd, thread) = open_settings();
    let hwnd = HWND(hwnd as _);
    let client = client(hwnd);
    // SAFETY: COM on this thread, then client calls on live elements.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let ua: IUIAutomation =
            CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).unwrap();
        let window = ua.ElementFromHandle(hwnd).unwrap();
        assert_eq!(window.CurrentName().unwrap(), "kit uia test");
        let mut seen = Vec::new();
        read(&ua.RawViewWalker().unwrap(), &window, &mut seen);
        let names: Vec<(i32, String)> = seen.iter().map(|(c, n, _)| (*c, n.clone())).collect();
        assert_eq!(names.len(), expected.len(), "{names:#?}");
        for e in &expected {
            assert!(names.contains(e), "{e:?} missing");
        }

        // The window scrolls: Save in the band shows, the General card is under it.
        let save = find(&seen, UIA_ButtonControlTypeId, "Save");
        assert!(!save.CurrentIsOffscreen().unwrap().as_bool());
        assert!(
            within(&bounds(&save), &client),
            "Save inside the client area"
        );
        let start = find(&seen, UIA_CheckBoxControlTypeId, "Start with Windows");
        assert!(
            start.CurrentIsOffscreen().unwrap().as_bool(),
            "under the band"
        );
        assert_eq!(bounds(&start).width, 0.0, "no bounds while hidden");
        let sun = find(&seen, UIA_CheckBoxControlTypeId, "Sun");
        assert!(within(&bounds(&sun), &client));

        // Text a stepper cannot take is refused, and the window lives on.
        let every = find(&seen, UIA_SpinnerControlTypeId, "Every (minutes)");
        let value: IUIAutomationValuePattern =
            every.GetCurrentPatternAs(UIA_ValuePatternId).unwrap();
        for text in ["é1", "1€", "ab€", "", "08:30"] {
            assert!(
                value.SetValue(&windows::core::BSTR::from(text)).is_err(),
                "{text:?}"
            );
        }
        assert_eq!(value.CurrentValue().unwrap(), "30");
        value.SetValue(&windows::core::BSTR::from("45")).unwrap();
        assert_eq!(value.CurrentValue().unwrap(), "45");

        // The patterns reach the real form.
        let fri = find(&seen, UIA_CheckBoxControlTypeId, "Fri");
        let toggle: IUIAutomationTogglePattern =
            fri.GetCurrentPatternAs(UIA_TogglePatternId).unwrap();
        toggle.Toggle().unwrap();
        assert_eq!(toggle.CurrentToggleState().unwrap(), ToggleState_On);

        // Focus on the hidden switch scrolls it into view first; the snapshot the focus
        // event goes out with already has it showing.
        start.SetFocus().unwrap();
        assert!(!start.CurrentIsOffscreen().unwrap().as_bool());
        assert!(within(&bounds(&start), &client), "in view after the focus");
        let told = kept(hwnd).unwrap();
        let key = told.tree.focus().unwrap();
        assert_eq!(told.tree.get(key).unwrap().name, "Start with Windows");
        assert!(
            !told.offscreen(key),
            "showing when the focus event went out"
        );
        assert!(sun.CurrentIsOffscreen().unwrap().as_bool(), "scrolled away");
    }
    quit(hwnd, thread);
    assert!(kept(hwnd).is_none(), "gone with its window");
}

/// The tray flyout (issue #89) on a real popup kit window: the window is named "breakpoint" by
/// its caption, though it shows none, and the switch is named "Nudges" with its note as help.
#[test]
fn a_ui_automation_client_reads_the_tray_flyout() {
    use crate::platform::flyout::{Content, items};

    let _one = one_client();
    let (tx, rx) = mpsc::channel();
    let thread = thread::spawn(move || {
        let content = Content {
            on: true,
            status: "On until 17:00".into(),
            note: "Turn off until tomorrow 08:00.".into(),
            pending: None,
            taken: 7,
            skipped: 2,
        };
        let spec = Spec {
            caption: "breakpoint".into(),
            width: crate::platform::theme::SIZE_FLYOUT,
            height: 0,
            items: items(&content),
            at: Some(POINT { x: 0, y: 0 }),
            owner: None,
            frame: window::Frame::Popup,
        };
        let toggled = std::rc::Rc::new(std::cell::Cell::new(false));
        let seen = toggled.clone();
        let handle = window::open(spec, move |_, event| {
            if matches!(event, Event::Changed(_, Value::On(false))) {
                seen.set(true);
            }
        })
        .unwrap();
        tx.send(handle.hwnd().0 as isize).unwrap();
        let mut msg = MSG::default();
        // SAFETY: a plain message loop on this thread.
        unsafe {
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        handle.close();
        assert!(toggled.get(), "the client's Toggle flipped the switch");
    });
    let hwnd = HWND(rx.recv().unwrap() as _);
    // SAFETY: COM on this thread, then client calls on live elements.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let ua: IUIAutomation =
            CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).unwrap();
        let window = ua.ElementFromHandle(hwnd).unwrap();
        assert_eq!(window.CurrentName().unwrap(), "breakpoint");
        let mut seen = Vec::new();
        read(&ua.RawViewWalker().unwrap(), &window, &mut seen);
        let switch = find(&seen, UIA_CheckBoxControlTypeId, "Nudges");
        assert_eq!(
            switch.CurrentHelpText().unwrap(),
            "Turn off until tomorrow 08:00."
        );
        find(&seen, UIA_ButtonControlTypeId, "Settings…");
        find(&seen, UIA_ButtonControlTypeId, "Quit");
        let toggle: IUIAutomationTogglePattern =
            switch.GetCurrentPatternAs(UIA_TogglePatternId).unwrap();
        assert_eq!(toggle.CurrentToggleState().unwrap(), ToggleState_On);
        toggle.Toggle().unwrap();
    }
    quit(hwnd, thread);
}
