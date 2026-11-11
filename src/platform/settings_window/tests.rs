//! The settings window's layout, keys, profile and Save, on a hidden kit window where they need
//! one. The ignored tests show the window or a message box; `BP_SHOTS=<folder>` makes the
//! `shots` test write BMPs (see there).

use std::path::Path;
use std::time::Duration;

use breakpoint::core::config::Config;
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Gdi::{
    BitBlt, EnumDisplayMonitors, GetDC, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO, ReleaseDC,
    SRCCOPY,
};
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForMonitor, GetDpiForWindow,
    MDT_EFFECTIVE_DPI, SetThreadDpiAwarenessContext,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    VIRTUAL_KEY, VK_DOWN, VK_ESCAPE, VK_RETURN, VK_SPACE, VK_TAB,
};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowExW, GetClientRect, GetWindowRect, SWP_NOSIZE, SWP_NOZORDER, SendMessageW,
    SetWindowPos, WM_CHAR, WM_CLOSE, WM_KEYDOWN,
};
use windows::core::{BOOL, w};

use super::*;
use crate::platform::kit::draw::{GdiMeasure, paint_form, paint_on};
use crate::platform::kit::form::Form as Laid;
use crate::platform::kit::shots::{capture, pump, render};
use crate::platform::kit::window::menu_of;
use crate::platform::theme::{self, Colours, Fonts, Theme, high_contrast_black, px};

const SCALES: [u32; 4] = [96, 120, 144, 192];

fn defaults() -> Form {
    Form::new(&Config::default())
}

/// The window's items laid out at `dpi` with the real fonts.
fn laid_out(dpi: u32) -> Laid {
    let fonts = Fonts::new(dpi);
    let mut form = Laid::new(items(&defaults(), true));
    unsafe {
        let dc = GetDC(None);
        form.lay_out(dpi, &GdiMeasure { dc, fonts: &fonts });
        ReleaseDC(None, dc);
    }
    form
}

fn overlap(a: &RECT, b: &RECT) -> bool {
    a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom
}

fn inside(inner: &RECT, outer: &RECT) -> bool {
    inner.left >= outer.left
        && inner.right <= outer.right
        && inner.top >= outer.top
        && inner.bottom <= outer.bottom
}

/// The ids in Tab order, as the kit's form steps through them.
fn tab_order(form: &Laid) -> Vec<Id> {
    form.tab_stops()
        .into_iter()
        .map(|i| form.items()[i].id)
        .collect()
}

/// Tab stops in reading order: the week, the profile, each nudge's row left to right with its
/// switch first, Start with Windows, then Save and Cancel.
fn reading_order() -> Vec<Id> {
    let rows = [&STAND, &PAUSE].map(|r| [r.on, r.every, r.length, r.level]);
    (0..7)
        .map(|i| DAY + i)
        .chain([FROM, TO, PROFILE])
        .chain(rows.into_iter().flatten())
        .chain([WATER_ON, WATER_EVERY, START_WITH_WINDOWS, SAVE, CANCEL])
        .collect()
}

#[test]
fn tab_runs_in_the_order_the_page_reads() {
    let form = Laid::new(items(&defaults(), true));
    assert_eq!(tab_order(&form), reading_order());
    assert_eq!(form.focused(), Some(DAY), "the focus starts on Sunday");
}

/// Every control, text and glyph keeps clear of the others, stays inside the window's padding,
/// and sits inside one card row, on the band, or above the cards as a heading, at every scale.
#[test]
fn items_keep_apart_inside_their_cards_at_every_scale() {
    for dpi in SCALES {
        let form = laid_out(dpi);
        let s = |v: i32| px(v, dpi);
        let all: Vec<(&Item, RECT, &[i32])> = form
            .items()
            .iter()
            .zip(form.laid())
            .map(|(item, laid)| (item, laid.rect, laid.rows.as_slice()))
            .collect();
        let boxes: Vec<_> = all
            .iter()
            .filter(|(item, ..)| matches!(item.kind, Kind::Card { .. } | Kind::Band))
            .collect();
        let leaves: Vec<_> = all
            .iter()
            .filter(|(item, ..)| !matches!(item.kind, Kind::Card { .. } | Kind::Band))
            .collect();
        for (k, (item, r, _)) in leaves.iter().enumerate() {
            let what = format!("{:?} at {dpi}: {r:?}", item.kind);
            assert!(r.left >= s(PAD) && r.right <= s(WIDTH - PAD), "{what}");
            let holder = boxes.iter().find(|(_, b, _)| overlap(r, b));
            if let Some((outer, b, rows)) = holder {
                assert!(inside(r, b), "{what} crosses its box {b:?}");
                if matches!(outer.kind, Kind::Card { .. }) {
                    assert!(
                        r.left >= b.left + s(SPACE_4) && r.right <= b.right - s(SPACE_4),
                        "{what} is in the card's side padding"
                    );
                    for &line in *rows {
                        assert!(
                            r.bottom <= line || r.top >= line,
                            "{what} crosses a hairline"
                        );
                    }
                }
            }
            for (other, o, _) in &leaves[k + 1..] {
                assert!(!overlap(r, o), "{what} overlaps {:?} {o:?}", other.kind);
                let same_line = r.top < o.bottom && o.top < r.bottom;
                if same_line {
                    let gap = (o.left - r.right).max(r.left - o.right);
                    assert!(gap >= s(SPACE_1) - 1, "{what} is {gap}px from {o:?}");
                }
            }
        }
        // The client height is the band's bottom.
        let band = boxes
            .iter()
            .find(|(item, ..)| item.kind == Kind::Band)
            .unwrap();
        assert_eq!(form.height(0), band.1.bottom, "at {dpi}");
    }
}

#[test]
fn save_is_the_default_and_cancel_the_cancel_button_at_the_right() {
    let all = items(&defaults(), true);
    let button = |id| all.iter().find(|item| item.id == id).unwrap();
    let (save, cancel) = (button(SAVE), button(CANCEL));
    assert!(matches!(
        save.kind,
        Kind::Button {
            kind: ButtonKind::Primary,
            default: true,
            cancel: false,
            ..
        }
    ));
    assert!(matches!(
        cancel.kind,
        Kind::Button {
            kind: ButtonKind::Secondary,
            default: false,
            cancel: true,
            ..
        }
    ));
    assert_eq!(cancel.at.x + cancel.at.w, WIDTH - PAD);
    assert_eq!(cancel.at.x - (save.at.x + save.at.w), SPACE_2);
}

#[test]
fn the_profile_menu_offers_the_presets_and_custom_only_in_the_field() {
    let mut form = defaults();
    let drop_down = |form: &Form| {
        items(form, true)
            .into_iter()
            .find(|item| item.id == PROFILE)
            .unwrap()
            .kind
    };
    let Kind::DropDown {
        entries,
        current,
        other,
    } = drop_down(&form)
    else {
        panic!("the profile is a drop-down");
    };
    let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["Gentle", "Balanced", "Strict"]);
    assert_eq!(current, Some(1), "Balanced by default");
    assert_eq!(other, "Custom");
    let note = entries[1].note.as_deref().unwrap();
    assert!(note.starts_with("Stand every 30 min"), "{note}");
    assert!(!note.contains("Water"), "{note}");
    form.stand.every = "35".into();
    assert!(matches!(
        drop_down(&form),
        Kind::DropDown { current: None, .. }
    ));
}

// ---- On a hidden window ----

/// Opens the settings window hidden, filled with `form`, with no owner.
fn open_hidden(form: &Form) -> Handle {
    create(form, true, Some(POINT { x: 0, y: 0 })).expect("the window opens")
}

fn key(handle: Handle, vk: VIRTUAL_KEY) {
    unsafe {
        SendMessageW(
            handle.hwnd(),
            WM_KEYDOWN,
            Some(WPARAM(vk.0.into())),
            Some(LPARAM(0)),
        );
    }
}

fn type_text(handle: Handle, text: &str) {
    for c in text.chars() {
        unsafe {
            SendMessageW(
                handle.hwnd(),
                WM_CHAR,
                Some(WPARAM(c as usize)),
                Some(LPARAM(0)),
            );
        }
    }
}

#[test]
fn tab_on_the_window_runs_in_reading_order_and_wraps() {
    let handle = open_hidden(&defaults());
    let mut got = vec![handle.focused().unwrap()];
    for _ in 0..reading_order().len() {
        key(handle, VK_TAB);
        got.push(handle.focused().unwrap());
    }
    let mut want = reading_order();
    want.push(DAY);
    assert_eq!(got, want);
    handle.close();
}

#[test]
fn picking_a_profile_fills_the_numbers_and_leaves_the_switches() {
    let mut form = defaults();
    form.pause.on = false;
    let handle = open_hidden(&form);
    let strict = Profile::ALL
        .iter()
        .position(|&p| p == Profile::Strict)
        .unwrap();
    on_event(handle, Event::Changed(PROFILE, Value::Index(Some(strict))));
    let read = read(handle);
    let mut want = form.clone();
    want.apply(Profile::Strict);
    assert_eq!(read, want);
    assert!(!read.pause.on, "the switch stays off");
    assert_eq!(handle.value(PROFILE), Some(Value::Index(Some(strict))));
    handle.close();
}

#[test]
fn typing_a_number_no_preset_has_shows_custom_and_back_shows_the_profile() {
    let handle = open_hidden(&defaults());
    handle.focus(STAND.every);
    type_text(handle, "35");
    key(handle, VK_TAB);
    assert_eq!(handle.value(STAND.every), Some(Value::Number(35)));
    assert_eq!(handle.value(PROFILE), Some(Value::Index(None)), "Custom");
    handle.focus(STAND.every);
    key(handle, VK_DOWN);
    assert_eq!(handle.value(STAND.every), Some(Value::Number(30)));
    assert_eq!(
        handle.value(PROFILE),
        Some(Value::Index(Some(1))),
        "Balanced"
    );
    // A level is part of the profile too.
    handle.focus(STAND.level);
    key(
        handle,
        windows::Win32::UI::Input::KeyboardAndMouse::VK_RIGHT,
    );
    assert_eq!(handle.value(PROFILE), Some(Value::Index(None)));
    handle.close();
}

#[test]
fn enter_saves_what_was_typed_and_esc_cancels() {
    // Enter while typing in a stepper takes the number, then saves.
    let handle = open_hidden(&defaults());
    handle.focus(STAND.every);
    type_text(handle, "45");
    key(handle, VK_RETURN);
    let saved = take_saved().expect("Enter saved");
    assert_eq!(saved.settings.nudges.stand.every_minutes, 45);
    assert!(saved.start_with_windows);
    assert!(!handle.is_open());
    assert!(current().is_none());

    // Esc cancels: nothing saved, the window gone.
    let handle = open_hidden(&defaults());
    key(handle, VK_ESCAPE);
    assert!(take_saved().is_none(), "Esc saved nothing");
    assert!(!handle.is_open());

    // Space on Save saves too; on Cancel it cancels.
    let handle = open_hidden(&defaults());
    handle.focus(CANCEL);
    key(handle, VK_SPACE);
    assert!(take_saved().is_none());
    assert!(!handle.is_open());
}

#[test]
fn opened_at_a_point_its_top_left_is_there() {
    // Change… opens the window where the wizard's top-left was.
    let at = POINT { x: 60, y: 50 };
    let handle = create(&defaults(), true, Some(at)).unwrap();
    let r = handle.window_rect();
    assert_eq!((r.left, r.top), (at.x, at.y));
    let mut client = RECT::default();
    unsafe { GetClientRect(handle.hwnd(), &mut client).unwrap() };
    let dpi = unsafe { GetDpiForWindow(handle.hwnd()) };
    assert_eq!(client.right, px(WIDTH, dpi));
    assert_eq!(client.bottom, laid_out(dpi).height(0));
    handle.close();
}

// ---- Shown ----

fn activate(handle: Handle) {
    handle.show();
    pump(100);
}

/// Closes the next "breakpoint settings" message box.
fn close_message_box() -> std::thread::JoinHandle<()> {
    std::thread::spawn(|| {
        for _ in 0..200 {
            std::thread::sleep(Duration::from_millis(25));
            if let Ok(found) =
                unsafe { FindWindowExW(None, None, w!("#32770"), w!("breakpoint settings")) }
            {
                unsafe { PostMessageW(Some(found), WM_CLOSE, WPARAM(0), LPARAM(0)) }.unwrap();
                return;
            }
        }
        panic!("no message box");
    })
}

#[test]
#[ignore = "shows the window and message boxes"]
fn every_refused_save_focuses_its_control() {
    let cases: [(Field, &dyn Fn(Handle)); 7] = [
        (Field::WorkDays, &|h| {
            (0..7).for_each(|i| {
                h.set(DAY + i, Value::On(false));
            })
        }),
        (Field::WorkHoursEnd, &|h| {
            h.set(TO, Value::Number(7 * 60));
        }),
        (Field::StandEvery, &|h| {
            h.set(STAND.every, Value::Number(5));
        }),
        (Field::StandLength, &|h| {
            h.set(STAND.length, Value::Number(9));
        }),
        (Field::PauseEvery, &|h| {
            h.set(PAUSE.every, Value::Number(999));
        }),
        (Field::PauseLength, &|h| {
            h.set(PAUSE.length, Value::Number(20));
        }),
        (Field::WaterEvery, &|h| {
            h.set(WATER_EVERY, Value::Number(1));
        }),
    ];
    for (field, spoil) in cases {
        let handle = open_hidden(&defaults());
        activate(handle);
        spoil(handle);
        handle.focus(START_WITH_WINDOWS);
        let closer = close_message_box();
        key(handle, VK_RETURN);
        closer.join().unwrap();
        assert_eq!(handle.focused(), Some(control_of(field)), "{field:?}");
        assert!(handle.is_open(), "{field:?}: the window stays");
        assert!(take_saved().is_none());
        handle.close();
    }
}

/// The monitors' work areas and DPIs.
fn monitors() -> Vec<(RECT, u32)> {
    unsafe extern "system" fn found(monitor: HMONITOR, _: HDC, _: *mut RECT, all: LPARAM) -> BOOL {
        unsafe {
            let mut info = MONITORINFO {
                cbSize: size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            let _ = GetMonitorInfoW(monitor, &mut info);
            let (mut x, mut y) = (96, 96);
            let _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut x, &mut y);
            (*(all.0 as *mut Vec<(RECT, u32)>)).push((info.rcWork, x));
        }
        BOOL(1)
    }
    let mut all = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(found), LPARAM(&mut all as *mut _ as _));
    }
    all
}

#[test]
#[ignore = "shows the window; needs two monitors at different scales"]
fn moving_to_a_monitor_at_another_scale_lays_out_again() {
    unsafe {
        SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    let handle = open_hidden(&defaults());
    activate(handle);
    let first = unsafe { GetDpiForWindow(handle.hwnd()) };
    let Some((work, dpi)) = monitors().into_iter().find(|&(_, dpi)| dpi != first) else {
        eprintln!("every monitor is at {first} DPI");
        handle.close();
        return;
    };
    let flags = SWP_NOSIZE | SWP_NOZORDER;
    unsafe {
        SetWindowPos(
            handle.hwnd(),
            None,
            work.left + 40,
            work.top + 40,
            0,
            0,
            flags,
        )
    }
    .unwrap();
    pump(300);
    assert_eq!(unsafe { GetDpiForWindow(handle.hwnd()) }, dpi);
    let mut client = RECT::default();
    unsafe { GetClientRect(handle.hwnd(), &mut client).unwrap() };
    assert_eq!(
        (client.right, client.bottom),
        (px(WIDTH, dpi), laid_out(dpi).height(0)),
        "from {first} to {dpi}"
    );
    if let Ok(folder) = std::env::var("BP_SHOTS") {
        capture(
            handle.hwnd(),
            &Path::new(&folder).join(format!("settings-moved-{first}-to-{dpi}.bmp")),
        );
    }
    handle.close();
}

/// For a look by eye: `BP_SHOTS=<folder> cargo test settings_window::tests::shots -- --ignored`
/// writes the settings window to BMPs there: light, dark and high contrast (the High Contrast
/// Black colours) at 100, 125, 150 and 200 %, focus ring on Sunday. Then it opens the window
/// in the current theme at the monitor's scale and captures it with its frame, then with the
/// profile menu open, and the menu itself.
#[test]
#[ignore = "writes BMPs and shows the window"]
fn shots() {
    let Ok(folder) = std::env::var("BP_SHOTS") else {
        eprintln!("set BP_SHOTS to a folder for the shots");
        return;
    };
    let folder = std::path::PathBuf::from(folder);
    unsafe {
        SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    crate::platform::paint::startup();
    let schemes = [
        ("light", Colours::of(Theme::Light)),
        ("dark", Colours::of(Theme::Dark)),
        ("contrast", Colours::high_contrast(high_contrast_black)),
    ];
    for (name, c) in schemes {
        for dpi in SCALES {
            let fonts = Fonts::new(dpi);
            let mut form = laid_out(dpi);
            form.show_cues();
            form.focus_in();
            let (w, h) = (px(WIDTH, dpi), form.height(0));
            let path = folder.join(format!("settings-{name}-{dpi}.bmp"));
            render(w, h, &path, |dc| unsafe {
                let looks = form.looks();
                paint_on(dc, w, h, c.ground, &fonts, dpi, |canvas, m, runs| {
                    paint_form(canvas, &form, &looks, &c, m, runs);
                });
            });
        }
    }

    let handle = open_hidden(&defaults());
    activate(handle);
    pump(300);
    capture(handle.hwnd(), &folder.join("settings-live.bmp"));
    handle.focus(PROFILE);
    key(handle, VK_SPACE);
    pump(300);
    capture(handle.hwnd(), &folder.join("settings-live-open.bmp"));
    let menu = menu_of(handle).expect("the menu is open");
    capture(menu, &folder.join("settings-live-menu.bmp"));
    // The menu is wider than the field at the window's right: their right edges line up, so
    // it stays over the window. The screen grab shows both.
    let mut m = RECT::default();
    unsafe { GetWindowRect(menu, &mut m).unwrap() };
    let window = handle.window_rect();
    assert!(
        m.right <= window.right && m.left >= window.left,
        "{m:?} in {window:?}"
    );
    render(
        window.right - window.left,
        window.bottom - window.top,
        &folder.join("settings-live-screen.bmp"),
        |dc| unsafe {
            let screen = GetDC(None);
            let _ = BitBlt(
                dc,
                0,
                0,
                window.right - window.left,
                window.bottom - window.top,
                Some(screen),
                window.left,
                window.top,
                SRCCOPY,
            );
            ReleaseDC(None, screen);
        },
    );
    eprintln!(
        "live: theme {:?}, high contrast {}, {} DPI",
        theme::app_theme(),
        theme::high_contrast(),
        unsafe { GetDpiForWindow(handle.hwnd()) }
    );
    handle.close();
}
