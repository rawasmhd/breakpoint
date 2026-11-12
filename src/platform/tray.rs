//! The notification icon: its On, Pending and Quiet states and its tooltip (SPEC §7, §8), and
//! the clicks that open and close the tray flyout ([`super::flyout`], issue #89).
//!
//! `tray-icon` runs on the app's own message loop and has no menu. A left or right click opens
//! the flyout; a click while it is open closes it. `tray-icon` registers its icon at the legacy
//! notify-icon version, at which Windows reports Enter, Space and the menu key on a focused icon
//! (Win+B, then the arrows) as a right-button click too; such a click opens the flyout with the
//! focus ring shown, since the pointer is not on the icon.
//!
//! What the user asks for, from the icon or from the flyout, waits in one queue as [`Command`]s
//! for the main loop, which acts on them with nothing borrowed.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::error::Error;

use breakpoint::core::quiet::SwitchItem;
use breakpoint::core::scheduler::Prompt;
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use windows::Win32::Foundation::RECT;
use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSMICON, SM_CYSMICON};

use super::flyout;

/// The exe's icon resources (see `build.rs`), one per [`Look`].
const ON_ICON: u16 = 1;
const QUIET_ICON: u16 = 2;
const PENDING_ICON: u16 = 3;

/// What the user asked for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Command {
    /// A click on the icon: open the flyout next to `icon` (its rectangle on screen), or close
    /// it if it is open. `keyboard` when it came from the keyboard rather than the pointer.
    Toggle {
        icon: RECT,
        keyboard: bool,
    },
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
    /// A press on the icon came while the flyout was open, or just after it closed: that
    /// press closed it, so its release must not open it again.
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

/// One press or release on the icon, as far as the flyout cares.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Click {
    /// A button went down, or a double click's second press.
    Press,
    /// A button came up, at `icon` on screen; `keyboard` when the pointer was not on the icon.
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
                button: MouseButton::Left | MouseButton::Right,
                ..
            } => return Some(Click::Press),
            _ => return None,
        };
        if !up {
            return Some(Click::Press);
        }
        let icon = RECT {
            left: rect.position.x.round() as i32,
            top: rect.position.y.round() as i32,
            right: rect.position.x.round() as i32 + rect.size.width as i32,
            bottom: rect.position.y.round() as i32 + rect.size.height as i32,
        };
        let (x, y) = (position.x.round() as i32, position.y.round() as i32);
        let on_icon = x >= icon.left && x < icon.right && y >= icon.top && y < icon.bottom;
        Some(Click::Release {
            icon,
            keyboard: !on_icon,
        })
    }
}

/// What a click does, given whether the flyout is open or has just closed (`busy`) and whether
/// the press before it swallows its release. Returns the command to queue, if any, and whether
/// the next release is swallowed.
///
/// A press on the icon while the flyout is open takes the activation away from it first, so
/// the flyout has closed, or is about to, by the time the release comes: that release must
/// not open it again. A release with no swallowing press toggles the flyout.
fn on_click(click: Click, busy: bool, swallow: bool) -> (Option<Command>, bool) {
    match click {
        Click::Press => (None, swallow || busy),
        Click::Release { .. } if swallow => (None, false),
        Click::Release { icon, keyboard } => (Some(Command::Toggle { icon, keyboard }), false),
    }
}

fn on_event(event: TrayIconEvent) {
    let Some(click) = Click::of(&event) else {
        return;
    };
    let busy = flyout::busy();
    let (command, swallow) = on_click(click, busy, SWALLOW_RELEASE.get());
    SWALLOW_RELEASE.set(swallow);
    if let Some(command) = command {
        push(command);
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

    fn release(keyboard: bool) -> Click {
        Click::Release {
            icon: ICON,
            keyboard,
        }
    }

    #[test]
    fn a_click_with_the_flyout_closed_opens_it() {
        let (none, swallow) = on_click(Click::Press, false, false);
        assert_eq!((none, swallow), (None, false));
        assert_eq!(
            on_click(release(false), false, swallow),
            (
                Some(Command::Toggle {
                    icon: ICON,
                    keyboard: false
                }),
                false
            )
        );
    }

    #[test]
    fn a_click_that_closed_the_flyout_does_not_open_it_again() {
        // The press took the activation away: the flyout closed just before it came.
        let (none, swallow) = on_click(Click::Press, true, false);
        assert_eq!((none, swallow), (None, true));
        assert_eq!(on_click(release(false), false, swallow), (None, false));
        // The next click opens it.
        let (_, swallow) = on_click(Click::Press, false, false);
        assert!(matches!(
            on_click(release(false), false, swallow),
            (Some(Command::Toggle { .. }), false)
        ));
    }

    #[test]
    fn a_double_click_toggles_once_per_release() {
        // Press, release (opens), second press (closes it by activation), release.
        let (_, s) = on_click(Click::Press, false, false);
        let (open, s) = on_click(release(false), false, s);
        assert!(open.is_some());
        let (_, s) = on_click(Click::Press, true, s);
        assert_eq!(on_click(release(false), false, s), (None, false));
    }

    #[test]
    fn a_keyboard_release_says_so() {
        assert_eq!(
            on_click(release(true), false, false).0,
            Some(Command::Toggle {
                icon: ICON,
                keyboard: true
            })
        );
    }
}
