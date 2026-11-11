//! The tray-icon menu and icon states.
//!
//! So far the off switch with its tooltip, the On, Pending and Quiet icons (SPEC §7, §8), the
//! pending section (§8 item 3), the Today count (§8 item 4), Settings… and Quit. `tray-icon` runs on the app's own message loop.

use std::error::Error;

use breakpoint::core::quiet::SwitchItem;
use breakpoint::core::scheduler::Prompt;
use tray_icon::menu::{IsMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder, TrayIconEvent};
use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSMICON, SM_CYSMICON};

/// The exe's icon resources (see `build.rs`), one per [`Look`].
const ON_ICON: u16 = 1;
const QUIET_ICON: u16 = 2;
const PENDING_ICON: u16 = 3;

/// Where the pending section goes: under the off switch and its separator, above Today.
const PENDING_AT: usize = 2;

/// A menu item the user picked.
pub enum Command {
    /// The off switch at the top (SPEC §7).
    Switch,
    Snooze,
    Skip,
    /// Settings… (SPEC §8 item 6).
    Settings,
    Quit,
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

/// The icon in the notification area; dropping it removes the icon.
pub struct Tray {
    icon: TrayIcon,
    /// The icon for each [`Look`], in its order.
    icons: [Icon; 3],
    /// What the icon shows now, to skip updates that change nothing.
    shown_look: Option<Look>,
    /// Quiet or Off, as the off switch last said.
    quiet: bool,
    /// An occurrence is pending, as the pending section last said.
    has_pending: bool,
    menu: Menu,
    switch: MenuItem,
    /// What the off switch and tooltip show now, to skip updates that change nothing.
    shown_switch: Option<SwitchItem>,
    /// The disabled line naming the pending occurrence, e.g. "Pause pending".
    pending: MenuItem,
    snooze: MenuItem,
    skip: MenuItem,
    /// The disabled Today count line, e.g. "Today: 7 taken, 2 skipped".
    today: MenuItem,
    settings: MenuId,
    /// How many items of the pending section are in the menu now, all at [`PENDING_AT`].
    shown: usize,
    quit: MenuId,
}

impl Tray {
    pub fn new() -> Result<Self, Box<dyn Error>> {
        // Clicks and hovers are not used; a handler keeps them from piling up in a channel.
        TrayIconEvent::set_event_handler(Some(|_| {}));

        let switch = MenuItem::new("", true, None);
        let today = MenuItem::new("", false, None);
        let settings = MenuItem::new("Settings…", true, None);
        let quit = MenuItem::new("Quit", true, None);
        let menu = Menu::new();
        menu.append_items(&[
            &switch,
            &PredefinedMenuItem::separator(),
            &today,
            &PredefinedMenuItem::separator(),
            &settings,
            &quit,
        ])?;
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
            .with_menu(Box::new(menu.clone()))
            .build()?;
        Ok(Self {
            icon,
            icons,
            shown_look: Some(Look::On),
            quiet: false,
            has_pending: false,
            menu,
            switch,
            shown_switch: None,
            pending: MenuItem::new("", false, None),
            snooze: MenuItem::new("Snooze 5 min", true, None),
            skip: MenuItem::new("Skip", true, None),
            today,
            settings: settings.id().clone(),
            shown: 0,
            quit: quit.id().clone(),
        })
    }

    /// Shows the off switch's label and the tooltip (SPEC §7 table), and the Quiet icon while
    /// quiet.
    pub fn set_switch(&mut self, item: &SwitchItem) {
        self.quiet = item.greyed;
        self.show_look();
        if self.shown_switch.as_ref() == Some(item) {
            return;
        }
        self.switch.set_text(item.label);
        // A failed update leaves the old tooltip; the next change tries again.
        let ok = self.icon.set_tooltip(Some(&item.tooltip)).is_ok();
        self.shown_switch = ok.then(|| item.clone());
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

    /// Shows the pending section for `prompt`, or removes it with nothing pending. Snooze is
    /// left out once used. The icon shows Pending while `prompt` is there, unless quiet.
    pub fn set_pending(&mut self, prompt: Option<&Prompt>) {
        self.has_pending = prompt.is_some();
        self.show_look();
        for _ in 0..self.shown {
            self.menu.remove_at(PENDING_AT);
        }
        let Some(prompt) = prompt else {
            self.shown = 0;
            return;
        };
        self.pending
            .set_text(format!("{} pending", prompt.nudge.name()));
        let mut items: Vec<&dyn IsMenuItem> = vec![&self.pending];
        if prompt.snooze {
            items.push(&self.snooze);
        }
        items.push(&self.skip);
        self.shown = items
            .iter()
            .enumerate()
            .take_while(|&(i, item)| self.menu.insert(*item, PENDING_AT + i).is_ok())
            .count();
    }

    /// Shows the Today count line (SPEC §10.3). Only a change touches the menu.
    pub fn set_today(&self, text: &str) {
        if self.today.text() != text {
            self.today.set_text(text);
        }
    }

    /// The next menu item picked since the last call, if any. Call after each dispatched message.
    pub fn next_command(&self) -> Option<Command> {
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            if event.id == self.quit {
                return Some(Command::Quit);
            } else if event.id == *self.switch.id() {
                return Some(Command::Switch);
            } else if event.id == *self.snooze.id() {
                return Some(Command::Snooze);
            } else if event.id == *self.skip.id() {
                return Some(Command::Skip);
            } else if event.id == self.settings {
                return Some(Command::Settings);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::Look;

    #[test]
    fn quiet_wins_over_pending_and_pending_over_on() {
        assert_eq!(Look::of(false, false), Look::On);
        assert_eq!(Look::of(false, true), Look::Pending);
        assert_eq!(Look::of(true, false), Look::Quiet);
        assert_eq!(Look::of(true, true), Look::Quiet);
    }
}
