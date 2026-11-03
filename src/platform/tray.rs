//! The tray-icon menu and icon states.
//!
//! So far the normal icon and one item, Quit. `tray-icon` runs on the app's own message loop.

use std::error::Error;

use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder, TrayIconEvent};
use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSMICON, SM_CYSMICON};

/// The exe's icon resource for the normal state (see `build.rs`).
const NORMAL_ICON: u16 = 1;

/// A menu item the user picked.
pub enum Command {
    Quit,
}

/// The icon in the notification area; dropping it removes the icon.
pub struct Tray {
    _icon: TrayIcon,
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
            .with_menu(Box::new(menu))
            .build()?;
        Ok(Self {
            _icon: tray,
            quit: quit.id().clone(),
        })
    }

    /// The next menu item picked since the last call, if any. Call after each dispatched message.
    pub fn next_command(&self) -> Option<Command> {
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            if event.id == self.quit {
                return Some(Command::Quit);
            }
        }
        None
    }
}
