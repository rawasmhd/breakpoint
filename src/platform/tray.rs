//! The notification icon: its On, Pending and Quiet states and its tooltip (SPEC §7, §8), and
//! the clicks that open and close the tray flyout ([`super::flyout`], issue #89).
//!
//! `tray-icon` runs on the app's own message loop and has no menu. A left or right click opens
//! the flyout; a click while it is open closes it. The icon's event handler does this itself,
//! in `tray-icon`'s window procedure, so a click works at once also while a modal loop runs (a
//! message box, a window being dragged), as a native tray menu does.
//!
//! `tray-icon` 0.26 does not set `NOTIFYICON_VERSION_4`, so `NIN_SELECT`, `NIN_KEYSELECT` and
//! `WM_CONTEXTMENU` never come. At the legacy version Windows reports Enter, Space and the menu
//! key on a focused icon (Win+B, then the arrows) as a right-button press and release; such a
//! release opens the flyout with the focus ring shown, since the pointer is not on the icon.
//!
//! What the user picks in the flyout waits in a queue as [`Command`]s for the main loop, which
//! acts on them with nothing borrowed.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::error::Error;

use breakpoint::core::quiet::SwitchItem;
use breakpoint::core::scheduler::Prompt;
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use windows::Win32::Foundation::{POINT, RECT};
use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSMICON, SM_CYSMICON};

use super::flyout;

/// The exe's icon resources (see `build.rs`), one per [`Look`].
const ON_ICON: u16 = 1;
const QUIET_ICON: u16 = 2;
const PENDING_ICON: u16 = 3;

/// What the user picked in the flyout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    /// The flyout's Nudges switch (SPEC §7).
    Switch,
    Snooze,
    Skip,
    /// Settings… (SPEC §8).
    Settings,
    Quit,
}

thread_local! {
    static COMMANDS: RefCell<VecDeque<Command>> = const { RefCell::new(VecDeque::new()) };
    /// The last press on the icon closed the flyout, so its release must not open it again.
    /// Every press sets it afresh, so a press with no release (a drag of the icon) does not
    /// eat a later click.
    static SWALLOW_RELEASE: Cell<bool> = const { Cell::new(false) };
}

/// Queues `command` for [`Tray::next_command`].
pub fn push(command: Command) {
    COMMANDS.with_borrow_mut(|q| q.push_back(command));
}

/// The oldest queued command, if any.
pub(crate) fn take() -> Option<Command> {
    COMMANDS.with_borrow_mut(VecDeque::pop_front)
}

/// The three icon states, told apart by shape (SPEC §8, DESIGN §7).
#[derive(Clone, Copy, PartialEq, Debug)]
enum Look {
    /// A filled red dot: nudges can fire and nothing is pending.
    On,
    /// A smaller dot inside a ring: an occurrence is pending, popup shown or closed with ×.
    Pending,
    /// A hollow grey ring: quiet or Off.
    Quiet,
}

impl Look {
    /// Quiet wins over Pending; starting quiet expires what is pending anyway.
    fn of(quiet: bool, pending: bool) -> Self {
        if quiet {
            Look::Quiet
        } else if pending {
            Look::Pending
        } else {
            Look::On
        }
    }
}

/// One press or release on the icon at `icon` on screen, as far as the flyout cares.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Click {
    /// A button went down, or a double click's second press.
    Press { icon: RECT },
    /// A button came up; `keyboard` when the pointer was not on the icon.
    Release { icon: RECT, keyboard: bool },
}

impl Click {
    fn of(event: &TrayIconEvent) -> Option<Click> {
        let (rect, position, up) = match event {
            TrayIconEvent::Click {
                rect,
                position,
                button: MouseButton::Left | MouseButton::Right,
                button_state,
                ..
            } => (rect, position, *button_state == MouseButtonState::Up),
            TrayIconEvent::DoubleClick {
                rect,
                position,
                button: MouseButton::Left | MouseButton::Right,
                ..
            } => (rect, position, false),
            _ => return None,
        };
        let icon = RECT {
            left: rect.position.x.round() as i32,
            top: rect.position.y.round() as i32,
            right: rect.position.x.round() as i32 + rect.size.width as i32,
            bottom: rect.position.y.round() as i32 + rect.size.height as i32,
        };
        if !up {
            return Some(Click::Press { icon });
        }
        let at = POINT {
            x: position.x.round() as i32,
            y: position.y.round() as i32,
        };
        Some(Click::Release {
            icon,
            keyboard: !flyout::inside(&icon, at),
        })
    }
}

/// What a click on the icon does to the flyout.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Act {
    Nothing,
    Close,
    Open { icon: RECT, keyboard: bool },
}

/// What `click` does, given whether the flyout is `open`, whether it closed just now because
/// this same press took the activation away (`lost_here`), and whether the last press
/// swallows this release. Returns the act and whether the next release is swallowed.
///
/// A press on the icon takes the activation away from the flyout, so the flyout has closed by
/// the time the press is reported, or is about to: the press closes it if it is still open,
/// and its release must not open it again. A release with no swallowing press opens it.
fn on_click(click: Click, open: bool, lost_here: bool, swallow: bool) -> (Act, bool) {
    match click {
        Click::Press { .. } if open => (Act::Close, true),
        Click::Press { .. } => (Act::Nothing, lost_here),
        Click::Release { .. } if swallow => (Act::Nothing, false),
        Click::Release { icon, keyboard } => (Act::Open { icon, keyboard }, false),
    }
}

/// The icon's event handler, run in `tray-icon`'s window procedure on the UI thread.
fn on_event(event: TrayIconEvent) {
    let Some(click) = Click::of(&event) else {
        return;
    };
    let lost_here = match click {
        Click::Press { icon } => flyout::lost_at(&icon),
        Click::Release { .. } => false,
    };
    let (act, swallow) = on_click(click, flyout::is_open(), lost_here, SWALLOW_RELEASE.get());
    SWALLOW_RELEASE.set(swallow);
    match act {
        Act::Nothing => {}
        Act::Close => flyout::close(),
        Act::Open { icon, keyboard } => flyout::open(icon, keyboard),
    }
}

/// The icon in the notification area; dropping it removes the icon.
pub struct Tray {
    icon: TrayIcon,
    /// The icon for each [`Look`], in its order.
    icons: [Icon; 3],
    /// What the icon shows now, to skip updates that change nothing.
    shown_look: Option<Look>,
    /// Quiet or Off, as the switch item last said.
    quiet: bool,
    /// An occurrence is pending, as the scheduler last said.
    has_pending: bool,
    /// The tooltip shown now, to skip updates that change nothing.
    shown_tooltip: Option<String>,
}

impl Tray {
    pub fn new() -> Result<Self, Box<dyn Error>> {
        TrayIconEvent::set_event_handler(Some(on_event));
        // SAFETY: no pointers; plain metric queries.
        let size = unsafe { (GetSystemMetrics(SM_CXSMICON), GetSystemMetrics(SM_CYSMICON)) };
        let size = Some((size.0 as u32, size.1 as u32));
        let icons = [
            Icon::from_resource(ON_ICON, size)?,
            Icon::from_resource(PENDING_ICON, size)?,
            Icon::from_resource(QUIET_ICON, size)?,
        ];
        let icon = TrayIconBuilder::new()
            .with_icon(icons[Look::On as usize].clone())
            .with_tooltip("breakpoint")
            .build()?;
        Ok(Self {
            icon,
            icons,
            shown_look: Some(Look::On),
            quiet: false,
            has_pending: false,
            shown_tooltip: None,
        })
    }

    /// Shows the tooltip (SPEC §7 table), and the Quiet icon while quiet.
    pub fn set_switch(&mut self, item: &SwitchItem) {
        self.quiet = item.greyed;
        self.show_look();
        if self.shown_tooltip.as_ref() == Some(&item.tooltip) {
            return;
        }
        // A failed update leaves the old tooltip; the next change tries again.
        let ok = self.icon.set_tooltip(Some(&item.tooltip)).is_ok();
        self.shown_tooltip = ok.then(|| item.tooltip.clone());
    }

    /// Shows the icon for the state now. A failed update leaves the old icon; the next call
    /// tries again.
    fn show_look(&mut self) {
        let look = Look::of(self.quiet, self.has_pending);
        if self.shown_look == Some(look) {
            return;
        }
        let ok = self
            .icon
            .set_icon(Some(self.icons[look as usize].clone()))
            .is_ok();
        self.shown_look = ok.then_some(look);
    }

    /// The icon shows Pending while `prompt` is there, unless quiet.
    pub fn set_pending(&mut self, prompt: Option<&Prompt>) {
        self.has_pending = prompt.is_some();
        self.show_look();
    }

    /// The next command since the last call, if any. Call after each dispatched message.
    pub fn next_command(&self) -> Option<Command> {
        take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quiet_wins_over_pending_and_pending_over_on() {
        assert_eq!(Look::of(false, false), Look::On);
        assert_eq!(Look::of(false, true), Look::Pending);
        assert_eq!(Look::of(true, false), Look::Quiet);
        assert_eq!(Look::of(true, true), Look::Quiet);
    }

    const ICON: RECT = RECT {
        left: 900,
        top: 740,
        right: 924,
        bottom: 764,
    };
    const PRESS: Click = Click::Press { icon: ICON };
    const OPEN: Act = Act::Open {
        icon: ICON,
        keyboard: false,
    };

    fn release(keyboard: bool) -> Click {
        Click::Release {
            icon: ICON,
            keyboard,
        }
    }

    #[test]
    fn a_click_with_the_flyout_closed_opens_it() {
        let (act, swallow) = on_click(PRESS, false, false, false);
        assert_eq!((act, swallow), (Act::Nothing, false));
        assert_eq!(
            on_click(release(false), false, false, swallow),
            (OPEN, false)
        );
    }

    #[test]
    fn a_press_while_open_closes_it_and_its_release_does_nothing() {
        let (act, swallow) = on_click(PRESS, true, false, false);
        assert_eq!((act, swallow), (Act::Close, true));
        assert_eq!(
            on_click(release(false), false, false, swallow),
            (Act::Nothing, false)
        );
    }

    #[test]
    fn a_press_that_took_the_activation_away_does_not_open_it_again() {
        // The flyout closed on lost activation, at this icon, just before the press came.
        let (act, swallow) = on_click(PRESS, false, true, false);
        assert_eq!((act, swallow), (Act::Nothing, true));
        assert_eq!(
            on_click(release(false), false, false, swallow),
            (Act::Nothing, false)
        );
    }

    #[test]
    fn a_press_with_no_release_eats_no_later_click() {
        // A press that closed the flyout, then the icon dragged away: no release comes.
        let (_, swallow) = on_click(PRESS, true, false, false);
        assert!(swallow);
        // The next click is a press of its own, which sets the swallow afresh.
        let (_, swallow) = on_click(PRESS, false, false, swallow);
        assert_eq!(
            on_click(release(false), false, false, swallow),
            (OPEN, false)
        );
    }

    #[test]
    fn a_double_click_opens_then_closes() {
        let (_, s) = on_click(PRESS, false, false, false);
        let (act, s) = on_click(release(false), false, false, s);
        assert_eq!(act, OPEN);
        // The second press closes it, or finds it closed by that press.
        let (act, s) = on_click(PRESS, true, false, s);
        assert_eq!(act, Act::Close);
        assert_eq!(on_click(release(false), false, false, s).0, Act::Nothing);
    }

    #[test]
    fn a_keyboard_release_says_so() {
        assert_eq!(
            on_click(release(true), false, false, false).0,
            Act::Open {
                icon: ICON,
                keyboard: true
            }
        );
    }
}
