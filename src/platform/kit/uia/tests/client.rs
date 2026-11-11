//! The providers through UI Automation itself: a bare window on a thread of its own answers
//! `WM_GETOBJECT` with [`get_object`], as the kit window does, and a UI Automation client reads
//! and drives it from the test thread, as a screen reader would from its own process.

use std::sync::Once;
use std::sync::mpsc;
use std::thread;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, MSG, PostMessageW,
    PostQuitMessage, RegisterClassExW, WINDOW_EX_STYLE, WM_CLOSE, WM_DESTROY, WM_GETOBJECT,
    WNDCLASSEXW, WS_OVERLAPPEDWINDOW,
};
use windows::core::w;

use super::super::{get_object, gone};
use super::*;

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: plain window messages on the live test window.
    unsafe {
        match msg {
            WM_GETOBJECT => get_object(hwnd, wparam, lparam, &HOSTED)
                .unwrap_or_else(|| DefWindowProcW(hwnd, msg, wparam, lparam)),
            WM_DESTROY => {
                gone(hwnd);
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

/// A hidden window over `items` on a thread of its own, until it closes. Returns the window
/// as a number.
fn host(items: fn() -> Vec<Item>) -> (isize, thread::JoinHandle<()>) {
    static CLASS: Once = Once::new();
    let (tx, rx) = mpsc::channel();
    let thread = thread::spawn(move || {
        FORM.set(Some(laid(items(), 96)));
        // SAFETY: a plain window class and window; the loop runs until WM_QUIT.
        unsafe {
            let instance = GetModuleHandleW(None).unwrap();
            CLASS.call_once(|| {
                RegisterClassExW(&WNDCLASSEXW {
                    cbSize: size_of::<WNDCLASSEXW>() as u32,
                    lpfnWndProc: Some(proc),
                    hInstance: instance.into(),
                    lpszClassName: w!("breakpoint.uia.test"),
                    ..Default::default()
                });
            });
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("breakpoint.uia.test"),
                w!("uia test"),
                WS_OVERLAPPEDWINDOW,
                0,
                0,
                600,
                800,
                None,
                None,
                Some(instance.into()),
                None,
            )
            .unwrap();
            tx.send(hwnd.0 as isize).unwrap();
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                DispatchMessageW(&msg);
            }
        }
    });
    (rx.recv().unwrap(), thread)
}

/// The window frame's own parts, which Windows adds next to the providers.
fn frame_part(control: UIA_CONTROLTYPE_ID) -> bool {
    control == UIA_TitleBarControlTypeId || control == UIA_MenuBarControlTypeId
}

type Seen = Vec<(i32, String, IUIAutomationElement)>;

fn read(walker: &IUIAutomationTreeWalker, from: &IUIAutomationElement, out: &mut Seen) {
    // SAFETY: live elements; a missing child or sibling ends the walk.
    unsafe {
        let mut child = walker.GetFirstChildElement(from).ok();
        while let Some(c) = child {
            let control = c.CurrentControlType().unwrap();
            if !frame_part(control) {
                out.push((control.0, c.CurrentName().unwrap().to_string(), c.clone()));
                read(walker, &c, out);
            }
            child = walker.GetNextSiblingElement(&c).ok();
        }
    }
}

fn element(seen: &Seen, control: UIA_CONTROLTYPE_ID, name: &str) -> IUIAutomationElement {
    seen.iter()
        .find(|(c, n, _)| *c == control.0 && n == name)
        .map(|(.., e)| e.clone())
        .unwrap_or_else(|| panic!("no {name:?}"))
}

#[test]
fn a_ui_automation_client_reads_and_drives_the_settings_window() {
    let expected: Vec<(i32, String)> = tree(&settings())
        .nodes()
        .iter()
        .skip(1)
        .map(|n| (Element::control_type(n.role).0.0, n.name.clone()))
        .collect();
    let (hwnd, thread) = host(settings_items);
    let hwnd = HWND(hwnd as _);
    // SAFETY: COM on this thread, then client calls on live elements.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let ua: IUIAutomation =
            CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).unwrap();
        let window = ua.ElementFromHandle(hwnd).unwrap();
        assert_eq!(
            window.CurrentName().unwrap(),
            "uia test",
            "the frame names the root"
        );
        let mut seen = Vec::new();
        read(&ua.RawViewWalker().unwrap(), &window, &mut seen);
        let names: Vec<(i32, String)> = seen.iter().map(|(c, n, _)| (*c, n.clone())).collect();
        assert_eq!(names.len(), expected.len(), "{names:#?}");
        for e in &expected {
            assert!(names.contains(e), "{e:?} missing");
        }

        let fri = element(&seen, UIA_CheckBoxControlTypeId, "Fri");
        let toggle: IUIAutomationTogglePattern =
            fri.GetCurrentPatternAs(UIA_TogglePatternId).unwrap();
        assert_eq!(toggle.CurrentToggleState().unwrap(), ToggleState_Off);
        toggle.Toggle().unwrap();
        assert_eq!(toggle.CurrentToggleState().unwrap(), ToggleState_On);

        let start = element(&seen, UIA_CheckBoxControlTypeId, "Start with Windows");
        assert_eq!(
            start.CurrentLocalizedControlType().unwrap(),
            "toggle switch"
        );
        assert_eq!(
            start.CurrentHelpText().unwrap(),
            "breakpoint starts in the tray when you sign in."
        );

        let profile = element(&seen, UIA_ComboBoxControlTypeId, "Profile");
        let expand: IUIAutomationExpandCollapsePattern = profile
            .GetCurrentPatternAs(UIA_ExpandCollapsePatternId)
            .unwrap();
        assert_eq!(
            expand.CurrentExpandCollapseState().unwrap(),
            ExpandCollapseState_Collapsed
        );
        let value: IUIAutomationValuePattern =
            profile.GetCurrentPatternAs(UIA_ValuePatternId).unwrap();
        assert_eq!(value.CurrentValue().unwrap(), "Balanced");

        let every = element(&seen, UIA_SpinnerControlTypeId, "Every (minutes)");
        let range: IUIAutomationRangeValuePattern =
            every.GetCurrentPatternAs(UIA_RangeValuePatternId).unwrap();
        range.SetValue(35.0).unwrap();
        assert_eq!(range.CurrentValue().unwrap(), 35.0);

        let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
    }
    thread.join().unwrap();
}
