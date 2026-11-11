//! The wizard's timeline, layout, keys and every way it closes, on a hidden kit window where
//! they need one. The ignored tests show the window; `BP_SHOTS=<folder>` makes the `shots` test
//! write BMPs (see there).

use std::path::Path;

use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetDC, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO, ReleaseDC,
};
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForMonitor, GetDpiForWindow,
    MDT_EFFECTIVE_DPI, SetThreadDpiAwarenessContext,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    VIRTUAL_KEY, VK_DOWN, VK_ESCAPE, VK_RETURN, VK_SPACE, VK_TAB,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClientRect, SWP_NOSIZE, SWP_NOZORDER, SendMessageW, SetWindowPos, WM_CLOSE, WM_KEYDOWN,
};
use windows::core::BOOL;

use super::*;
use crate::platform::kit::draw::{GdiMeasure, paint_form, paint_on};
use crate::platform::kit::form::Form;
use crate::platform::kit::shots::{capture, pump, render};
use crate::platform::theme::{self, Fonts, Theme, high_contrast_black, px};

const SCALES: [u32; 4] = [96, 120, 144, 192];
/// The default work day, 08:00 to 17:00.
const DAY: u32 = 9 * 60;

#[test]
fn numbers_line_drops_the_name_and_the_card_puts_water_on_its_own_line() {
    let line = numbers_line(Profile::Balanced);
    assert!(line.starts_with("Stand every 30 min"), "{line}");
    for profile in Profile::ALL {
        assert!(!numbers_line(profile).contains(profile_name(profile)));
    }
    assert_eq!(
        card_note(Profile::Balanced),
        "Stand every 30 min, pause every 60 (Pause insists).\nWater rides along hourly."
    );
}

// ---- The timeline ----

fn minutes(profile: Profile, tick: Tick) -> Vec<u32> {
    ticks(profile, DAY)
        .into_iter()
        .filter(|&(_, t)| t == tick)
        .map(|(m, _)| m)
        .collect()
}

#[test]
fn each_timeline_follows_its_profile_s_cadences() {
    // Balanced: Stand every 30, Pause every 60, which takes the Stand at the same minute.
    assert_eq!(
        minutes(Profile::Balanced, Tick::Pause),
        (1..=8).map(|k| k * 60).collect::<Vec<_>>()
    );
    assert_eq!(
        minutes(Profile::Balanced, Tick::Stand),
        (0..9).map(|k| 30 + k * 60).collect::<Vec<_>>()
    );
    // Gentle: 45 and 90.
    assert_eq!(
        minutes(Profile::Gentle, Tick::Pause),
        [90, 180, 270, 360, 450]
    );
    assert_eq!(
        minutes(Profile::Gentle, Tick::Stand),
        [45, 135, 225, 315, 405, 495]
    );
    // Strict: 30 and 30, so every tick is a Pause.
    assert_eq!(
        minutes(Profile::Strict, Tick::Pause),
        (1..=17).map(|k| k * 30).collect::<Vec<_>>()
    );
    assert!(minutes(Profile::Strict, Tick::Stand).is_empty());
    // In order, and nothing at the day's ends.
    for profile in Profile::ALL {
        let all = ticks(profile, DAY);
        assert!(all.windows(2).all(|w| w[0].0 < w[1].0), "{profile:?}");
        assert!(all.iter().all(|&(m, _)| m > 0 && m < DAY));
    }
}

#[test]
fn ticks_sit_on_their_minutes_in_the_box_at_any_scale() {
    let b = Box2 {
        x: 10.0,
        y: 0.0,
        w: 150.0,
        h: 24.0,
    };
    let (base, all) = timeline(Profile::Balanced, DAY, b, 96);
    assert_eq!(
        base,
        Box2 {
            x: 10.0,
            y: 11.0,
            w: 150.0,
            h: 2.0
        }
    );
    let close = |a: f32, b: f32| (a - b).abs() < 0.001;
    let (stand, tick) = all[0];
    assert_eq!(tick, Tick::Stand);
    // Minute 30 of 540: 150 × 30 / 540 = 8.33 px in, the tick 3 wide centred on it.
    assert!(
        close(stand.x, 10.0 + 150.0 * 30.0 / 540.0 - 1.5),
        "{stand:?}"
    );
    assert!(close(stand.w, 3.0) && close(stand.h, 12.0) && close(stand.y, 6.0));
    let (pause, tick) = all[1];
    assert_eq!(tick, Tick::Pause);
    assert!(
        close(pause.x, 10.0 + 150.0 * 60.0 / 540.0 - 1.5),
        "{pause:?}"
    );
    assert!(close(pause.h, 20.0) && close(pause.y, 2.0));
    // At 200 % the sizes double and the minutes keep their share of the box.
    let wide = Box2 {
        x: 20.0,
        y: 0.0,
        w: 300.0,
        h: 48.0,
    };
    let (base, all) = timeline(Profile::Balanced, DAY, wide, 192);
    assert!(close(base.h, 4.0) && close(base.y, 22.0));
    let (last, _) = *all.last().unwrap();
    assert!(close(last.x + last.w / 2.0, 20.0 + 300.0 * 510.0 / 540.0));
    assert!(close(last.w, 6.0) && close(last.h, 24.0));
}

// ---- Layout ----

fn laid_out(dpi: u32) -> Form {
    let fonts = Fonts::new(dpi);
    let mut form = Form::new(items(&Picks::default()));
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

/// Every item keeps clear of the others and inside the window's padding; what sits on the
/// Work week card or the band stays inside it; the cards keep `space-2` apart.
#[test]
fn items_keep_apart_inside_the_padding_at_every_scale() {
    for dpi in SCALES {
        let form = laid_out(dpi);
        let all: Vec<(&Item, RECT)> = form
            .items()
            .iter()
            .zip(form.laid())
            .map(|(item, laid)| (item, laid.rect))
            .collect();
        let boxes: Vec<_> = all
            .iter()
            .filter(|(item, _)| matches!(item.kind, Kind::Card { .. } | Kind::Band))
            .collect();
        let leaves: Vec<_> = all
            .iter()
            .filter(|(item, _)| !matches!(item.kind, Kind::Card { .. } | Kind::Band))
            .collect();
        for (k, (item, r)) in leaves.iter().enumerate() {
            let what = format!("{:?} at {dpi}: {r:?}", item.kind);
            assert!(
                r.left >= px(PAD, dpi) && r.right <= px(WIDTH - PAD, dpi),
                "{what}"
            );
            if let Some((_, b)) = boxes.iter().find(|(_, b)| overlap(r, b)) {
                assert!(
                    r.left >= b.left
                        && r.right <= b.right
                        && r.top >= b.top
                        && r.bottom <= b.bottom,
                    "{what} crosses {b:?}"
                );
            }
            for (other, o) in &leaves[k + 1..] {
                assert!(!overlap(r, o), "{what} overlaps {:?} {o:?}", other.kind);
            }
        }
        let cards: Vec<RECT> = all
            .iter()
            .filter(|(item, _)| matches!(item.kind, Kind::Choice { .. }))
            .map(|&(_, r)| r)
            .collect();
        for pair in cards.windows(2) {
            assert_eq!(pair[1].top - pair[0].bottom, px(SPACE_2, dpi), "at {dpi}");
        }
        let band = boxes
            .iter()
            .find(|(item, _)| item.kind == Kind::Band)
            .unwrap();
        assert_eq!(form.height(0), band.1.bottom, "at {dpi}");
    }
}

#[test]
fn the_cards_are_one_tab_stop_then_change_the_checkbox_and_start() {
    let form = Form::new(items(&Picks::default()));
    let order: Vec<Id> = form
        .tab_stops()
        .into_iter()
        .map(|i| form.items()[i].id)
        .collect();
    let balanced = PROFILE + 1;
    assert_eq!(order, [balanced, CHANGE, START_WITH_WINDOWS, START]);
    assert_eq!(
        form.focused(),
        Some(balanced),
        "the focus starts on the pick"
    );
}

// ---- On a hidden window ----

fn open_hidden() -> Handle {
    create(&Picks::default(), Some(POINT { x: 0, y: 0 })).expect("the wizard opens")
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

fn started() -> Picks {
    match take_wizard() {
        Some(WizardDone::Start(picks)) => picks,
        Some(WizardDone::Change(..)) => panic!("Change…, not Start"),
        None => panic!("the wizard did not close"),
    }
}

#[test]
fn enter_esc_and_the_close_box_all_start_with_what_the_page_shows() {
    for how in ["Enter", "Esc", "×"] {
        let handle = open_hidden();
        match how {
            "Enter" => key(handle, VK_RETURN),
            "Esc" => key(handle, VK_ESCAPE),
            _ => unsafe {
                SendMessageW(handle.hwnd(), WM_CLOSE, None, None);
            },
        }
        let picks = started();
        assert_eq!(picks.profile, Profile::Balanced, "{how}");
        assert!(picks.start_with_windows, "{how}");
        assert!(!handle.is_open(), "{how}");
        assert!(current().is_none(), "{how}");
    }
}

#[test]
fn arrows_pick_a_profile_and_space_clears_the_checkbox() {
    let handle = open_hidden();
    key(handle, VK_DOWN);
    assert_eq!(handle.focused(), Some(PROFILE + 2), "Strict");
    key(handle, VK_TAB);
    key(handle, VK_TAB);
    assert_eq!(handle.focused(), Some(START_WITH_WINDOWS));
    key(handle, VK_SPACE);
    key(handle, VK_RETURN);
    let picks = started();
    assert_eq!(picks.profile, Profile::Strict);
    assert!(!picks.start_with_windows);
}

#[test]
fn change_hands_the_picks_on_with_the_wizard_s_top_left() {
    let handle = open_hidden();
    key(handle, VK_DOWN);
    let r = handle.window_rect();
    handle.focus(CHANGE);
    key(handle, VK_SPACE);
    match take_wizard() {
        Some(WizardDone::Change(picks, at)) => {
            assert_eq!(picks.profile, Profile::Strict);
            assert_eq!((at.x, at.y), (r.left, r.top));
        }
        _ => panic!("Change… did not hand the picks on"),
    }
    assert!(!handle.is_open());
}

#[test]
fn the_window_is_as_wide_as_settings_and_as_tall_as_its_content() {
    let handle = open_hidden();
    let mut client = RECT::default();
    unsafe { GetClientRect(handle.hwnd(), &mut client).unwrap() };
    let dpi = unsafe { GetDpiForWindow(handle.hwnd()) };
    assert_eq!(client.right, px(settings_window::WIDTH, dpi));
    assert_eq!(client.bottom, laid_out(dpi).height(0));
    handle.close();
}

// ---- Shown ----

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
#[ignore = "shows the wizard; needs two monitors at different scales"]
fn moving_to_a_monitor_at_another_scale_lays_out_again() {
    unsafe {
        SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    let handle = open_hidden();
    handle.show();
    pump(100);
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
            &Path::new(&folder).join(format!("wizard-moved-{first}-to-{dpi}.bmp")),
        );
    }
    handle.close();
}

/// For a look by eye: `BP_SHOTS=<folder> cargo test dialogs::tests::shots -- --ignored` writes
/// the wizard to BMPs there: light, dark and high contrast (the High Contrast Black colours) at
/// 100, 125, 150 and 200 %, focus ring on Balanced. Then it opens the wizard in the current
/// theme at the monitor's scale and captures it with its frame.
#[test]
#[ignore = "writes BMPs and shows the wizard"]
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
            let path = folder.join(format!("wizard-{name}-{dpi}.bmp"));
            render(w, h, &path, |dc| unsafe {
                let looks = form.looks();
                paint_on(dc, w, h, c.ground, &fonts, dpi, |canvas, m, runs| {
                    paint_form(canvas, &form, &looks, &c, m, runs);
                });
            });
        }
    }

    let handle = open_hidden();
    handle.show();
    pump(400);
    capture(handle.hwnd(), &folder.join("wizard-live.bmp"));
    eprintln!(
        "live: theme {:?}, high contrast {}, {} DPI",
        theme::app_theme(),
        theme::high_contrast(),
        unsafe { GetDpiForWindow(handle.hwnd()) }
    );
    handle.close();
}
