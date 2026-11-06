//! breakpoint: single-instance guard, message loop (with IsDialogMessage) and wiring.

#![windows_subsystem = "windows"]

mod platform;

use std::cell::RefCell;
use std::error::Error;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use breakpoint::core::config::{self, Config};
use breakpoint::core::log::Today;
use breakpoint::core::presence::{POLL_EVERY, Presence, Tracker};
use breakpoint::core::quiet::Quiet;
use breakpoint::core::scheduler::{Effect, Scheduler, away_left, countdown_text};
use breakpoint::core::settings::{self, Form};
use breakpoint::core::wizard::Picks;
use platform::dialogs::{
    Saved, WM_OPEN_SETTINGS, WM_SETTINGS_CANCELLED, WM_SETTINGS_SAVED, WM_WIZARD_DONE, WizardDone,
};
use platform::log::Log;
use platform::popup::{Popup, WM_POPUP_SKIP, WM_POPUP_SNOOZE};
use platform::presence::{Listener, Vetoes};
use platform::tray::{Command, Tray};
use windows::Win32::Foundation::{
    ERROR_ALREADY_EXISTS, GetLastError, HWND, LPARAM, LRESULT, WPARAM,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::WindowsAndMessaging::{
    ASFW_ANY, AllowSetForegroundWindow, CreateWindowExW, DefWindowProcW, DestroyWindow,
    DispatchMessageW, FindWindowW, GetMessageW, MSG, PostMessageW, RegisterClassW, SetTimer,
    TranslateMessage, WM_ENDSESSION, WM_TIMER, WNDCLASSW, WS_EX_TOOLWINDOW, WS_POPUP,
};
use windows::core::w;

/// The main window's class, which a second launch looks for (SPEC §11).
const MAIN_CLASS: windows::core::PCWSTR = w!("breakpoint.main");

const POLL_TIMER: usize = 1;
/// Refreshes the popup's away countdown while it is on screen.
const COUNTDOWN_TIMER: usize = 2;
const COUNTDOWN_EVERY: Duration = Duration::from_secs(1);

/// Everything the window procedure acts on. Lives on the UI thread only.
struct App {
    /// The origin of the core's timeline.
    started: Instant,
    /// The hidden main window, which a Settings Save is posted to.
    window: HWND,
    /// The one in-memory copy of the settings: read at start-up, changed by a Settings Save and
    /// by the off switch, and written whole to `config.json` by either.
    config: Config,
    /// Where `config.json` lives; `None` if `%APPDATA%` is unknown, and then nothing is saved.
    config_path: Option<PathBuf>,
    /// First run (SPEC §9.1): the wizard's picks while the wizard, or the settings window its
    /// Change… opened, is still up. Until then nothing nudges and nothing is saved.
    first_run: Option<Picks>,
    quiet: Quiet,
    scheduler: Scheduler,
    popup: Popup,
    presence: Tracker,
    vetoes: Vetoes,
    /// What the last poll said, which holds the popup's countdown under a veto.
    last_presence: Presence,
    tray: Tray,
    log: Log,
    today: Today,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

impl App {
    fn now(&self) -> Duration {
        self.started.elapsed()
    }

    fn poll(&mut self) {
        self.sync_quiet();
        let now = self.now();
        let last_input = platform::presence::last_input(now);
        for (at, signal) in platform::presence::take_signals() {
            let at = at.saturating_duration_since(self.started);
            self.presence.signal(at, last_input, signal);
        }
        let presence = self.presence.poll(now, last_input, &mut self.vetoes);
        self.last_presence = presence;
        // During first run no clock runs: the first poll after it starts them from zero.
        if self.first_run.is_none() {
            let effects = self.scheduler.poll(now, presence);
            self.apply(effects);
        }
        // Midnight starts the count again (SPEC §10.3).
        self.tray
            .set_today(&self.today.text(platform::clock::now()));
    }

    /// Applies the quiet rules as of the wall clock now (SPEC §7): an edge expires what is
    /// pending or restarts the clocks. Then the tray's off switch, tooltip and icon follow.
    fn sync_quiet(&mut self) {
        let wall = platform::clock::now();
        let now = self.now();
        let effects = self.quiet.sync(wall, &mut self.scheduler, now);
        self.apply(effects);
        self.tray.set_switch(&self.quiet.item(wall));
    }

    /// The user pressed the off switch: start or end Off or Overtime, save its end moment in
    /// `config.json` so a restart keeps it, and apply the quiet rules at once.
    fn press_switch(&mut self) {
        self.quiet.press(platform::clock::now());
        self.quiet.store(&mut self.config);
        if let Some(path) = &self.config_path {
            // Best effort: if the write fails the switch still holds until breakpoint quits.
            let _ = config::save(path, &self.config);
        }
        self.sync_quiet();
    }

    /// Settings…, or a second launch: opens the settings window filled from the config, or
    /// brings it to the front (SPEC §9.2, §11). During first run it brings the wizard (or the
    /// settings window its Change… opened) to the front instead.
    fn open_settings(&mut self) {
        if self.first_run.is_some() {
            platform::dialogs::to_front();
            return;
        }
        platform::dialogs::open_settings(
            self.window,
            &Form::new(&self.config),
            platform::startup::is_on(),
            None,
        );
    }

    /// The wizard closed (SPEC §9.1). Start, X and Esc accept its picks; Change… opens the
    /// settings window in its place, with the picks and defaults for the rest.
    fn wizard_done(&mut self) -> Option<String> {
        match platform::dialogs::take_wizard()? {
            WizardDone::Start(picks) => self.finish_first_run(picks),
            WizardDone::Change(picks, at) => {
                self.first_run = Some(picks);
                platform::dialogs::open_settings(
                    self.window,
                    &picks.form(),
                    picks.start_with_windows,
                    Some(at),
                );
                None
            }
        }
    }

    /// The settings window closed without a Save. After the wizard's Change… that finishes
    /// first run as closing the wizard would: its picks are written, the edits discarded.
    fn settings_cancelled(&mut self) -> Option<String> {
        let picks = self.first_run?;
        self.finish_first_run(picks)
    }

    /// Accepts the wizard's picks: written and applied as a Save of them would be.
    fn finish_first_run(&mut self, picks: Picks) -> Option<String> {
        self.apply_saved(Saved {
            settings: picks.settings(),
            start_with_windows: picks.start_with_windows,
        })
    }

    /// A valid Save closed the settings window. During first run that Save finishes it.
    fn settings_saved(&mut self) -> Option<String> {
        let saved = platform::dialogs::take_saved()?;
        self.apply_saved(saved)
    }

    /// Applies settings at once (SPEC §9.2), writes `config.json` and the Run value, and ends
    /// first run, so nudging starts. Returns what could not be written, to tell the user.
    fn apply_saved(&mut self, saved: Saved) -> Option<String> {
        let wall = platform::clock::now();
        let now = self.now();
        let effects = settings::save(
            &saved.settings,
            &mut self.config,
            &mut self.quiet,
            &mut self.scheduler,
            wall,
            now,
        );
        self.apply(effects);
        self.tray.set_switch(&self.quiet.item(wall));

        let mut failed = Vec::new();
        match &self.config_path {
            Some(path) => {
                if let Err(e) = config::save(path, &self.config) {
                    failed.push(format!("couldn't write config.json ({e})"));
                }
            }
            None => failed.push("couldn't find %APPDATA% to save config.json".into()),
        }
        if let Err(e) = platform::startup::set(saved.start_with_windows) {
            failed.push(format!("couldn't change Start with Windows ({e})"));
        }
        if self.first_run.take().is_some() {
            self.poll();
        }
        (!failed.is_empty()).then(|| {
            format!(
                "The new settings apply until breakpoint quits, but breakpoint {}.",
                failed.join(" and ")
            )
        })
    }

    fn snooze(&mut self) {
        let effects = self.scheduler.snooze(self.now());
        self.apply(effects);
    }

    fn skip(&mut self) {
        let effects = self.scheduler.skip(self.now());
        self.apply(effects);
    }

    /// breakpoint is quitting, from the tray or because Windows ends the session: the pending
    /// occurrences expire (cause `exit`) and are logged before it goes (SPEC §11).
    fn quit(&mut self) {
        let effects = self.scheduler.quit(self.now());
        self.apply(effects);
    }

    fn apply(&mut self, effects: Vec<Effect>) {
        if effects.is_empty() {
            return;
        }
        let wall = platform::clock::now();
        for effect in effects {
            match effect {
                Effect::Log(event) => {
                    self.log.append(wall, &event);
                    self.today.record(wall, &event);
                }
                Effect::Show(prompt) => self.popup.show(&prompt, &self.countdown(prompt.length)),
                Effect::Close => self.popup.hide(),
                Effect::ShowWater => self.popup.water(),
            }
        }
        self.tray.set_pending(self.scheduler.prompt().as_ref());
        self.tray.set_today(&self.today.text(wall));
    }

    /// The countdown line for an occurrence of `length` as of now (SPEC §6).
    fn countdown(&self, length: Duration) -> String {
        let now = self.now();
        let held = self.last_presence == Presence::AtDesk;
        countdown_text(away_left(
            length,
            now,
            platform::presence::last_input(now),
            held,
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
            // During first run the switch would save a config: it shows the wizard instead.
            Some(Command::Switch) if self.first_run.is_some() => self.open_settings(),
            Some(Command::Switch) => self.press_switch(),
            Some(Command::Snooze) => self.snooze(),
            Some(Command::Skip) => self.skip(),
            Some(Command::Settings) => self.open_settings(),
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
        open_settings_in_running_instance();
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

/// A second launch asks the running breakpoint to open its Settings, then exits (SPEC §11). It
/// lets that instance take the foreground, which a background process may not do on its own.
fn open_settings_in_running_instance() {
    // SAFETY: no pointers beyond the static class name.
    unsafe {
        if let Ok(window) = FindWindowW(MAIN_CLASS, None) {
            let _ = AllowSetForegroundWindow(ASFW_ANY);
            let _ = PostMessageW(Some(window), WM_OPEN_SETTINGS, WPARAM(0), LPARAM(0));
        }
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    // Moving the exe must not break autostart: an existing Run value gets the current path.
    platform::startup::refresh();

    // Read once here and never watched (SPEC §10.1). No file yet is first run, which shows the
    // wizard and runs on its preselected picks until it closes (SPEC §9.1). An unreadable file
    // is not first run: it runs on the defaults with the notice.
    let config_path = config_path();
    let loaded = config_path
        .as_deref()
        .and_then(|path| config::load(path, platform::clock::now()));
    let first_run = (config_path.is_some() && loaded.is_none()).then(Picks::default);
    let (config, notice) = match loaded {
        Some(loaded) => (loaded.config, loaded.notice),
        None => (first_run.unwrap_or_default().config(), false),
    };

    // The occurrence ids go on from the log, and the Today count is rebuilt from it (SPEC §10).
    let log = Log::new();
    let (last_occ, today) = log.start(platform::clock::now());
    let mut scheduler = Scheduler::new(config.nudges, minute());
    scheduler.continue_from(last_occ);

    let window = main_window()?;
    let popup = Popup::new(window)?;
    if notice {
        popup.notice();
    }
    APP.set(Some(App {
        started: Instant::now(),
        window,
        quiet: Quiet::new(&config),
        scheduler,
        config,
        config_path,
        first_run,
        popup,
        presence: Tracker::new(),
        vetoes: Vetoes::default(),
        last_presence: Presence::Active,
        tray: Tray::new()?,
        log,
        today,
    }));
    // Session and power events take effect at once rather than at the next poll.
    let listener = Listener::new(|| with_app(App::poll))?;
    with_app(App::poll);
    if let Some(picks) = first_run {
        platform::dialogs::open_wizard(window, picks);
    }
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
        // The wizard and the settings window take their keys (Tab, Enter, Esc) first.
        if !platform::dialogs::is_dialog_message(&msg) {
            unsafe {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        let mut running = true;
        with_app(|app| running = app.tray_command());
        if !running {
            break;
        }
    }

    drop(listener);
    APP.take();
    // SAFETY: `window` is the live message-only window.
    unsafe { DestroyWindow(window)? };
    Ok(())
}

/// `%APPDATA%\breakpoint\config.json`; its folder is created on the first save.
fn config_path() -> Option<PathBuf> {
    let app_data = std::env::var_os("APPDATA")?;
    Some(
        PathBuf::from(app_data)
            .join("breakpoint")
            .join("config.json"),
    )
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

/// Runs `f`, then shows what it says could not be written. The message box is shown outside
/// the app, so its modal loop does not hold up the polls.
fn report(f: impl FnOnce(&mut App) -> Option<String>) {
    let mut failed = None;
    with_app(|app| failed = f(app));
    if let Some(text) = failed {
        platform::dialogs::error(&text);
    }
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

/// The hidden window that owns the poll timer and receives the popup's buttons. Top-level, never
/// shown, rather than message-only: only top-level windows hear that Windows is ending the session.
fn main_window() -> windows::core::Result<HWND> {
    // SAFETY: plain Win32 window creation.
    unsafe {
        let instance = GetModuleHandleW(None)?.into();
        let class = MAIN_CLASS;
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance,
            lpszClassName: class,
            ..Default::default()
        });
        CreateWindowExW(
            WS_EX_TOOLWINDOW,
            class,
            w!("breakpoint"),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
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
        WM_OPEN_SETTINGS => with_app(App::open_settings),
        WM_SETTINGS_SAVED => report(App::settings_saved),
        WM_SETTINGS_CANCELLED => report(App::settings_cancelled),
        WM_WIZARD_DONE => report(App::wizard_done),
        // Windows is ending the session and may end the process once this returns.
        WM_ENDSESSION if wparam.0 != 0 => with_app(App::quit),
        // SAFETY: forwarding the arguments this procedure was called with.
        _ => return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
    LRESULT(0)
}
