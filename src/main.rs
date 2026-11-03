//! breakpoint: single-instance guard, message loop (with IsDialogMessage) and wiring.

#![windows_subsystem = "windows"]

mod platform;

use std::cell::RefCell;
use std::error::Error;
use std::time::{Duration, Instant};

use breakpoint::core::presence::{POLL_EVERY, Presence};
use breakpoint::core::scheduler::{Effect, Scheduler, away_left, countdown_text};
use platform::popup::{Popup, WM_POPUP_SKIP, WM_POPUP_SNOOZE};
use platform::tray::{Command, Tray};
use windows::Win32::Foundation::{
    ERROR_ALREADY_EXISTS, GetLastError, HWND, LPARAM, LRESULT, WPARAM,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, HWND_MESSAGE,
    MSG, RegisterClassW, SetTimer, TranslateMessage, WINDOW_EX_STYLE, WINDOW_STYLE, WM_TIMER,
    WNDCLASSW,
};
use windows::core::w;

const POLL_TIMER: usize = 1;
/// Refreshes the popup's away countdown while it is on screen.
const COUNTDOWN_TIMER: usize = 2;
const COUNTDOWN_EVERY: Duration = Duration::from_secs(1);

/// Everything the window procedure acts on. Lives on the UI thread only.
struct App {
    /// The origin of the core's timeline.
    started: Instant,
    scheduler: Scheduler,
    popup: Popup,
    tray: Tray,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

impl App {
    fn now(&self) -> Duration {
        self.started.elapsed()
    }

    fn poll(&mut self) {
        let now = self.now();
        let presence = Presence::from_last_input(now, platform::presence::last_input(now));
        let effects = self.scheduler.poll(now, presence);
        self.apply(effects);
    }

    fn snooze(&mut self) {
        let effects = self.scheduler.snooze(self.now());
        self.apply(effects);
    }

    fn skip(&mut self) {
        let effects = self.scheduler.skip(self.now());
        self.apply(effects);
    }

    fn quit(&mut self) {
        let effects = self.scheduler.quit(self.now());
        self.apply(effects);
    }

    fn apply(&mut self, effects: Vec<Effect>) {
        if effects.is_empty() {
            return;
        }
        for effect in effects {
            match effect {
                // The event log comes with its own issue; nothing is written yet.
                Effect::Log(_) => {}
                Effect::Show(prompt) => self.popup.show(&prompt, &self.countdown(prompt.length)),
                Effect::Close => self.popup.hide(),
            }
        }
        self.tray.set_pending(self.scheduler.prompt().as_ref());
    }

    /// The countdown line for an occurrence of `length` as of now (SPEC §6).
    fn countdown(&self, length: Duration) -> String {
        let now = self.now();
        // No veto holds it yet: `AtDesk` comes with the presence issue.
        countdown_text(away_left(
            length,
            now,
            platform::presence::last_input(now),
            false,
        ))
    }

    fn tick_countdown(&mut self) {
        if self.popup.is_visible()
            && let Some(prompt) = self.scheduler.prompt()
        {
            self.popup.set_countdown(&self.countdown(prompt.length));
        }
    }

    /// Acts on the tray item picked, if any; false once the user chose Quit.
    fn tray_command(&mut self) -> bool {
        match self.tray.next_command() {
            Some(Command::Snooze) => self.snooze(),
            Some(Command::Skip) => self.skip(),
            Some(Command::Quit) => {
                self.quit();
                return false;
            }
            None => {}
        }
        true
    }
}

fn main() {
    if another_instance_running() {
        return;
    }
    // With no console there is nowhere to report a start-up failure yet; exit quietly.
    let _ = run();
}

/// Takes the named single-instance mutex; true if another breakpoint already holds it.
fn another_instance_running() -> bool {
    // SAFETY: no pointers. The handle is never closed: exiting releases the mutex.
    unsafe {
        match CreateMutexW(None, false, w!("Local\\breakpoint.single-instance")) {
            Ok(_) => GetLastError() == ERROR_ALREADY_EXISTS,
            Err(_) => false,
        }
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let window = message_window()?;
    APP.set(Some(App {
        started: Instant::now(),
        scheduler: Scheduler::new(minute()),
        popup: Popup::new(window)?,
        tray: Tray::new()?,
    }));
    with_app(App::poll);
    // SAFETY: `window` is the live message-only window.
    unsafe {
        SetTimer(
            Some(window),
            POLL_TIMER,
            POLL_EVERY.as_millis() as u32,
            None,
        );
        SetTimer(
            Some(window),
            COUNTDOWN_TIMER,
            COUNTDOWN_EVERY.as_millis() as u32,
            None,
        );
    }

    let mut msg = MSG::default();
    // SAFETY: the standard message loop over a valid MSG.
    while unsafe { GetMessageW(&mut msg, None, 0, 0) }.0 > 0 {
        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        let mut running = true;
        with_app(|app| running = app.tray_command());
        if !running {
            break;
        }
    }

    APP.take();
    // SAFETY: `window` is the live message-only window.
    unsafe { DestroyWindow(window)? };
    Ok(())
}

/// How long one minute of the cadences is. Debug builds honour `BREAKPOINT_MINUTE_SECS` so
/// manual checks don't take half an hour.
fn minute() -> Duration {
    #[cfg(debug_assertions)]
    if let Some(minute) = std::env::var("BREAKPOINT_MINUTE_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .and_then(|secs| Duration::try_from_secs_f64(secs).ok())
        .filter(|d| !d.is_zero())
    {
        return minute;
    }
    Duration::from_secs(60)
}

fn with_app(f: impl FnOnce(&mut App)) {
    APP.with(|app| {
        // A message dispatched while the app is busy (none should be) is dropped, not a panic.
        if let Ok(mut app) = app.try_borrow_mut()
            && let Some(app) = app.as_mut()
        {
            f(app);
        }
    });
}

/// The hidden message-only window that owns the poll timer and receives the popup's buttons.
fn message_window() -> windows::core::Result<HWND> {
    // SAFETY: plain Win32 window creation.
    unsafe {
        let instance = GetModuleHandleW(None)?.into();
        let class = w!("breakpoint.main");
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance,
            lpszClassName: class,
            ..Default::default()
        });
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class,
            w!("breakpoint"),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            Some(instance),
            None,
        )
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_TIMER if wparam.0 == POLL_TIMER => with_app(App::poll),
        WM_TIMER if wparam.0 == COUNTDOWN_TIMER => with_app(App::tick_countdown),
        WM_POPUP_SNOOZE => with_app(App::snooze),
        WM_POPUP_SKIP => with_app(App::skip),
        // SAFETY: forwarding the arguments this procedure was called with.
        _ => return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
    LRESULT(0)
}
