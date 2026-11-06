//! The tray-icon menu and icon states.
//!
//! So far the off switch with its tooltip and normal or greyed icon (SPEC §7, §8 item 1), the
//! pending section (§8 item 3), the Today count (§8 item 4) and Quit. `tray-icon` runs on the app's own message loop.

use std::error::Error;

use breakpoint::core::quiet::SwitchItem;
use breakpoint::core::scheduler::Prompt;
use tray_icon::menu::{IsMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder, TrayIconEvent};
use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSMICON, SM_CYSMICON};

/// The exe's icon resources (see `build.rs`): the normal state, and the greyed one for quiet.
const NORMAL_ICON: u16 = 1;
const GREY_ICON: u16 = 2;

/// Where the pending section goes: under the off switch and its separator, above Today.
const PENDING_AT: usize = 2;

/// A menu item the user picked.
pub enum Command {
    /// The off switch at the top (SPEC §7).
    Switch,
    Snooze,
    Skip,
    Quit,
}

/// The icon in the notification area; dropping it removes the icon.
pub struct Tray {
    icon: TrayIcon,
    normal: Icon,
    grey: Icon,
    menu: Menu,
    switch: MenuItem,
    /// What the off switch, tooltip and icon show now, to skip updates that change nothing.
    shown_switch: Option<SwitchItem>,
    /// The disabled line naming the pending occurrence, e.g. "Pause pending".
    pending: MenuItem,
    snooze: MenuItem,
    skip: MenuItem,
    /// The disabled Today count line, e.g. "Today: 7 taken, 2 skipped".
    today: MenuItem,
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
        let quit = MenuItem::new("Quit", true, None);
        let menu = Menu::new();
        menu.append_items(&[
            &switch,
            &PredefinedMenuItem::separator(),
            &today,
            &PredefinedMenuItem::separator(),
            &quit,
        ])?;
        // SAFETY: no pointers; plain metric queries.
        let size = unsafe { (GetSystemMetrics(SM_CXSMICON), GetSystemMetrics(SM_CYSMICON)) };
        let size = Some((size.0 as u32, size.1 as u32));
        let normal = Icon::from_resource(NORMAL_ICON, size)?;
        let grey = Icon::from_resource(GREY_ICON, size)?;
        let icon = TrayIconBuilder::new()
            .with_icon(normal.clone())
            .with_tooltip("breakpoint")
            .with_menu(Box::new(menu.clone()))
            .build()?;
        Ok(Self {
            icon,
            normal,
            grey,
            menu,
            switch,
            shown_switch: None,
            pending: MenuItem::new("", false, None),
            snooze: MenuItem::new("Snooze 5 min", true, None),
            skip: MenuItem::new("Skip", true, None),
            today,
            shown: 0,
            quit: quit.id().clone(),
        })
    }

    /// Shows the off switch's label, the tooltip and the normal or greyed icon (SPEC §7 table).
    pub fn set_switch(&mut self, item: &SwitchItem) {
        if self.shown_switch.as_ref() == Some(item) {
            return;
        }
        self.switch.set_text(item.label);
        // A failed update leaves the old look; the next change tries again.
        let icon = if item.greyed {
            &self.grey
        } else {
            &self.normal
        };
        let ok = self.icon.set_icon(Some(icon.clone())).is_ok()
            && self.icon.set_tooltip(Some(&item.tooltip)).is_ok();
        self.shown_switch = ok.then(|| item.clone());
    }

    /// Shows the pending section for `prompt`, or removes it with nothing pending. Snooze is
    /// left out once used.
    pub fn set_pending(&mut self, prompt: Option<&Prompt>) {
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
            }
        }
        None
    }
}
