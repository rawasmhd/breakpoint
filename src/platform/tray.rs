//! The tray-icon menu and icon states.
//!
//! So far the normal icon, the pending section (SPEC §8 item 3) and Quit. `tray-icon` runs on
//! the app's own message loop.

use std::error::Error;

use breakpoint::core::scheduler::Prompt;
use tray_icon::menu::{IsMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder, TrayIconEvent};
use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSMICON, SM_CYSMICON};

/// The exe's icon resource for the normal state (see `build.rs`).
const NORMAL_ICON: u16 = 1;

/// A menu item the user picked.
pub enum Command {
    Snooze,
    Skip,
    Quit,
}

/// The icon in the notification area; dropping it removes the icon.
pub struct Tray {
    _icon: TrayIcon,
    menu: Menu,
    /// The disabled line naming the pending occurrence, e.g. "Pause pending".
    pending: MenuItem,
    snooze: MenuItem,
    skip: MenuItem,
    separator: PredefinedMenuItem,
    /// How many items of the pending section are in the menu now, all at its top.
    shown: usize,
    quit: MenuId,
}

impl Tray {
    pub fn new() -> Result<Self, Box<dyn Error>> {
        // Clicks and hovers are not used; a handler keeps them from piling up in a channel.
        TrayIconEvent::set_event_handler(Some(|_| {}));

        let quit = MenuItem::new("Quit", true, None);
        let menu = Menu::new();
        menu.append(&quit)?;
        // SAFETY: no pointers; plain metric queries.
        let size = unsafe { (GetSystemMetrics(SM_CXSMICON), GetSystemMetrics(SM_CYSMICON)) };
        let icon = Icon::from_resource(NORMAL_ICON, Some((size.0 as u32, size.1 as u32)))?;
        let tray = TrayIconBuilder::new()
            .with_icon(icon)
            .with_tooltip("breakpoint")
            .with_menu(Box::new(menu.clone()))
            .build()?;
        Ok(Self {
            _icon: tray,
            menu,
            pending: MenuItem::new("", false, None),
            snooze: MenuItem::new("Snooze 5 min", true, None),
            skip: MenuItem::new("Skip", true, None),
            separator: PredefinedMenuItem::separator(),
            shown: 0,
            quit: quit.id().clone(),
        })
    }

    /// Shows the pending section for `prompt`, or removes it with nothing pending. Snooze is
    /// left out once used.
    pub fn set_pending(&mut self, prompt: Option<&Prompt>) {
        for _ in 0..self.shown {
            self.menu.remove_at(0);
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
        items.extend([&self.skip as &dyn IsMenuItem, &self.separator]);
        self.shown = items
            .iter()
            .enumerate()
            .take_while(|&(i, item)| self.menu.insert(*item, i).is_ok())
            .count();
    }

    /// The next menu item picked since the last call, if any. Call after each dispatched message.
    pub fn next_command(&self) -> Option<Command> {
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            if event.id == self.quit {
                return Some(Command::Quit);
            } else if event.id == *self.snooze.id() {
                return Some(Command::Snooze);
            } else if event.id == *self.skip.id() {
                return Some(Command::Skip);
            }
        }
        None
    }
}
