//! PROTOTYPE, throwaway. breakpoint's own small popup as the nudge channel.
//! Raw Win32 through the `windows` crate (the UI-technology decision).
//!
//! popup stand|pause|water [--soft] [--len S] [--repeat S] [--snooze S] [--auto]
//!
//! Seconds stand in for minutes: --len 20 is the stand length (2 min),
//! --repeat 20 is the Firm repeat (5 active min), --snooze 15 is Snooze (5 min).
//! The countdown is the real rule: away time measured from the last input,
//! restarting on any keyboard or mouse input; reaching 0 satisfies the nudge.
//! --auto drives the buttons itself (X, then Snooze, then Skip) with BM_CLICK
//! so the wiring can be checked without a human.

#![allow(static_mut_refs)]

use std::time::Instant;
use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::Media::Audio::{PlaySoundW, SND_ALIAS, SND_ASYNC};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::SystemInformation::GetTickCount;
use windows::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForSystem, SystemParametersInfoForDpi};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
use windows::Win32::UI::WindowsAndMessaging::*;

const ID_SNOOZE: i32 = 101;
const ID_SKIP: i32 = 102;

struct St {
    t0: Instant,
    kind: String,
    firm: bool,
    len_ms: u64,
    repeat_ms: u64,
    snooze_ms: u64,
    auto: bool,
    hwnd: HWND,
    countdown: HWND,
    snooze_btn: HWND,
    skip_btn: HWND,
    next_repeat: u64,
    snooze_until: Option<u64>,
    snoozed: bool,
    last_text: String,
    auto_step: usize,
}

static mut S: Option<St> = None;

fn st() -> &'static mut St {
    unsafe { S.as_mut().unwrap() }
}

fn ms() -> u64 {
    st().t0.elapsed().as_millis() as u64
}

fn log(m: &str) {
    println!("[{:>5.1}s] {m}", ms() as f32 / 1000.0);
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let kind = args.get(1).cloned().unwrap_or_else(|| "stand".into());
    let flag = |f: &str| args.iter().any(|a| a == f);
    let num = |f: &str, d: u64| {
        args.iter()
            .position(|a| a == f)
            .and_then(|i| args.get(i + 1))
            .and_then(|v| v.parse().ok())
            .unwrap_or(d)
    };
    let default_len = if kind == "pause" { 30 } else { 20 };
    unsafe {
        S = Some(St {
            t0: Instant::now(),
            firm: !flag("--soft"),
            len_ms: num("--len", default_len) * 1000,
            repeat_ms: num("--repeat", 20) * 1000,
            snooze_ms: num("--snooze", 15) * 1000,
            auto: flag("--auto"),
            kind,
            hwnd: HWND::default(),
            countdown: HWND::default(),
            snooze_btn: HWND::default(),
            skip_btn: HWND::default(),
            next_repeat: 0,
            snooze_until: None,
            snoozed: false,
            last_text: String::new(),
            auto_step: 0,
        });
        run().unwrap();
    }
}

unsafe fn run() -> windows::core::Result<()> {
    let s = st();
    let hinst: HINSTANCE = GetModuleHandleW(None)?.into();
    let class = w!("bp_popup_proto");
    RegisterClassW(&WNDCLASSW {
        lpfnWndProc: Some(wndproc),
        hInstance: hinst,
        lpszClassName: class,
        hCursor: LoadCursorW(None, IDC_ARROW)?,
        hbrBackground: HBRUSH(GetStockObject(WHITE_BRUSH).0),
        ..Default::default()
    });

    let dpi = GetDpiForSystem();
    let px = |v: i32| v * dpi as i32 / 96;
    let water = s.kind == "water";
    let (cw, ch) = (px(340), px(if water { 70 } else { 150 }));
    let style = WS_POPUP | WS_CAPTION | WS_SYSMENU;
    let ex = WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE;
    let mut r = RECT { left: 0, top: 0, right: cw, bottom: ch };
    AdjustWindowRectExForDpi(&mut r, style, false, ex, dpi)?;
    let (ww, wh) = (r.right - r.left, r.bottom - r.top);
    let mut wa = RECT::default();
    SystemParametersInfoW(SPI_GETWORKAREA, 0, Some(&mut wa as *mut _ as _), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0))?;
    let (x, y) = (wa.right - ww - px(12), wa.bottom - wh - px(12));

    let hwnd = CreateWindowExW(ex, class, w!("breakpoint"), style, x, y, ww, wh, None, None, Some(hinst), None)?;
    s.hwnd = hwnd;

    // System message font, scaled for this DPI, plus a bold copy for the title.
    let mut ncm = NONCLIENTMETRICSW { cbSize: std::mem::size_of::<NONCLIENTMETRICSW>() as u32, ..Default::default() };
    SystemParametersInfoForDpi(SPI_GETNONCLIENTMETRICS.0, ncm.cbSize, Some(&mut ncm as *mut _ as _), 0, dpi)?;
    let font = CreateFontIndirectW(&ncm.lfMessageFont);
    let mut lf = ncm.lfMessageFont;
    lf.lfWeight = 700;
    lf.lfHeight = lf.lfHeight * 6 / 5;
    let bold = CreateFontIndirectW(&lf);

    let (title, body) = match s.kind.as_str() {
        "pause" => ("Pause", "Focus period over. Step away for a few minutes."),
        "water" => ("Water", "Have a glass of water."),
        _ => ("Stand up and move", "Shoulders down, look at something far away."),
    };
    let child = |cls: PCWSTR, text: &str, st_: u32, x: i32, y: i32, w: i32, h: i32, id: i32, f: HFONT| -> HWND {
        let c = CreateWindowExW(
            WINDOW_EX_STYLE(0), cls, &HSTRING::from(text), WS_CHILD | WS_VISIBLE | WINDOW_STYLE(st_),
            px(x), px(y), px(w), px(h), Some(hwnd), Some(HMENU(id as _)), Some(hinst), None,
        ).unwrap();
        SendMessageW(c, WM_SETFONT, Some(WPARAM(f.0 as usize)), Some(LPARAM(1)));
        c
    };
    child(w!("STATIC"), title, 0, 14, 10, 312, 24, 0, bold);
    child(w!("STATIC"), body, 0, 14, 36, 312, 22, 0, font);
    if !water {
        s.countdown = child(w!("STATIC"), "", 0, 14, 62, 312, 40, 0, font);
        s.snooze_btn = child(w!("BUTTON"), "Snooze 5 min", BS_PUSHBUTTON as u32 | WS_TABSTOP.0, 140, 110, 104, 30, ID_SNOOZE, font);
        s.skip_btn = child(w!("BUTTON"), "Skip", BS_PUSHBUTTON as u32 | WS_TABSTOP.0, 252, 110, 76, 30, ID_SKIP, font);
    }

    pop(!water, "SHOWN");
    s.next_repeat = ms() + s.repeat_ms;
    SetTimer(Some(hwnd), 1, 250, None);
    log(&format!(
        "kind={} level={} len={}s repeat={}s snooze={}s at ({x},{y}) {ww}x{wh} dpi={dpi}",
        s.kind, if s.firm { "Firm" } else { "Soft" }, s.len_ms / 1000, s.repeat_ms / 1000, s.snooze_ms / 1000
    ));

    let mut msg = MSG::default();
    while GetMessageW(&mut msg, None, 0, 0).as_bool() {
        let _ = TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
    Ok(())
}

/// Show (or re-show) without stealing focus, on top, with the default sound.
unsafe fn pop(sound: bool, what: &str) {
    let s = st();
    let _ = ShowWindow(s.hwnd, SW_SHOWNOACTIVATE);
    let _ = SetWindowPos(s.hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW);
    if sound {
        let _ = PlaySoundW(w!("Notification.Default"), None, SND_ALIAS | SND_ASYNC);
    }
    let fg = GetForegroundWindow() == s.hwnd;
    log(&format!("{what}: visible, topmost, sound={sound}, stole focus={fg}"));
}

fn idle_ms() -> u64 {
    let mut lii = LASTINPUTINFO { cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
    unsafe {
        let _ = GetLastInputInfo(&mut lii);
        GetTickCount().wrapping_sub(lii.dwTime) as u64
    }
}

unsafe fn tick() {
    let s = st();
    let now = ms();
    if s.kind == "water" {
        if now >= 7000 {
            log("water timed out after 7 s (logged as shown)");
            let _ = DestroyWindow(s.hwnd);
        }
        return;
    }
    // Live countdown: away time from the last input. Runs while hidden or snoozed too.
    let left = s.len_ms.saturating_sub(idle_ms());
    let secs = left.div_ceil(1000);
    let text = format!(
        "Away {}:{:02} more and it counts. Any input starts it over.{}",
        secs / 60,
        secs % 60,
        if s.snoozed { "\nSnoozed once already: Skip only." } else { "" }
    );
    if text != s.last_text {
        let _ = SetWindowTextW(s.countdown, &HSTRING::from(&text));
        s.last_text = text;
    }
    if left == 0 {
        log("SATISFIED: away for the full length (detection, no button)");
        let _ = DestroyWindow(s.hwnd);
        return;
    }
    if let Some(u) = s.snooze_until {
        if now >= u {
            s.snooze_until = None;
            s.snoozed = true;
            let _ = ShowWindow(s.snooze_btn, SW_HIDE);
            pop(true, "RE-PROMPT after snooze (Skip only)");
            s.next_repeat = now + s.repeat_ms;
        }
    } else if s.firm && now >= s.next_repeat {
        pop(true, "FIRM REPEAT (same window, unchanged)");
        s.next_repeat = now + s.repeat_ms;
    }
    if s.auto {
        let steps: [(u64, &str); 3] = [(3000, "close"), (11000, "snooze"), (19000, "skip")];
        if let Some(&(at, what)) = steps.get(s.auto_step) {
            if now >= at {
                s.auto_step += 1;
                log(&format!("auto: pressing {what}"));
                match what {
                    "close" => { let _ = PostMessageW(Some(s.hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)); }
                    "snooze" => { SendMessageW(s.snooze_btn, BM_CLICK, None, None); }
                    _ => { SendMessageW(s.skip_btn, BM_CLICK, None, None); }
                }
            }
        }
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_TIMER => {
            tick();
            LRESULT(0)
        }
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_CTLCOLORSTATIC => {
            SetBkColor(HDC(wp.0 as _), COLORREF(0x00FF_FFFF));
            LRESULT(GetStockObject(WHITE_BRUSH).0 as isize)
        }
        WM_COMMAND => {
            let s = st();
            match (wp.0 & 0xFFFF) as i32 {
                ID_SNOOZE => {
                    log(&format!("CLICK reached app: Snooze -> pending, hidden, re-prompt in {}s", s.snooze_ms / 1000));
                    s.snooze_until = Some(ms() + s.snooze_ms);
                    let _ = ShowWindow(hwnd, SW_HIDE);
                }
                ID_SKIP => {
                    log("CLICK reached app: Skip -> occurrence SKIPPED");
                    let _ = DestroyWindow(hwnd);
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            log("closed with X, no button -> hidden, still PENDING");
            let _ = ShowWindow(hwnd, SW_HIDE);
            LRESULT(0)
        }
        WM_DPICHANGED => {
            let r = &*(lp.0 as *const RECT);
            let _ = SetWindowPos(hwnd, None, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}
