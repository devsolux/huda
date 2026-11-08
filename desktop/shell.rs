//! Native tray lifetime and window actions, owned only by the desktop/main thread.
use eframe::egui;
use huda::error::{Error, Result};
use std::sync::mpsc::{self, Receiver};
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder, TrayIconEvent};

pub enum Action {
    Show,
    Ask,
    Settings,
    Startup,
    Quit,
}
pub struct Tray {
    _icon: TrayIcon,
    events: Receiver<Action>,
    startup: CheckMenuItem,
}
impl Tray {
    /// Call only from App::logic after the native event loop is running (especially macOS).
    pub fn create(context: &egui::Context) -> Result<Self> {
        let menu = Menu::new();
        let show = MenuItem::with_id("show", "Show Huda", true, None);
        let ask = MenuItem::with_id("ask", "Ask Huda", true, None);
        let startup = CheckMenuItem::with_id("startup", "Launch at login", false, false, None);
        let quit = MenuItem::with_id("quit", "Quit Huda", true, None);
        menu.append_items(&[
            &show,
            &ask,
            &PredefinedMenuItem::separator(),
            &startup,
            &PredefinedMenuItem::separator(),
            &quit,
        ])
        .map_err(|_| Error::new("TRAY_UNAVAILABLE", "Cannot create the desktop tray menu"))?;
        let pixels =
            eframe::icon_data::from_png_bytes(include_bytes!("../assets/icons/huda-64.png"))
                .map_err(|_| Error::new("ICON_INVALID", "Embedded Huda icon is invalid"))?;
        let icon = Icon::from_rgba(pixels.rgba, pixels.width, pixels.height)
            .map_err(|_| Error::new("ICON_INVALID", "Tray icon dimensions are invalid"))?;
        let icon = TrayIconBuilder::new()
            .with_id(huda::APP_ID)
            .with_menu(Box::new(menu))
            .with_icon(icon)
            .with_tooltip(huda::APP_NAME)
            .build()
            .map_err(|_| {
                Error::new(
                    "TRAY_UNAVAILABLE",
                    "No usable desktop tray backend; the window remains accessible",
                )
            })?;
        let (sender, events) = mpsc::sync_channel(32);
        let wake = context.clone();
        let tray_sender = sender.clone();
        TrayIconEvent::set_event_handler(Some(move |event| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: tray_icon::MouseButton::Left,
                    button_state: tray_icon::MouseButtonState::Up,
                    ..
                } | TrayIconEvent::DoubleClick { .. }
            ) {
                let _ = tray_sender.try_send(Action::Show);
                wake.request_repaint();
            }
        }));
        let wake = context.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            let action = match event.id.0.as_str() {
                "show" => Action::Show,
                "ask" => Action::Ask,
                "settings" => Action::Settings,
                "startup" => Action::Startup,
                "quit" => Action::Quit,
                _ => return,
            };
            let _ = sender.try_send(action);
            wake.request_repaint();
        }));
        Ok(Self {
            _icon: icon,
            events,
            startup,
        })
    }
    pub fn drain(&self) -> Vec<Action> {
        self.events.try_iter().collect()
    }
    pub fn startup_state(&self, state: Option<bool>, busy: bool) {
        self.startup.set_checked(state.unwrap_or(false));
        self.startup.set_enabled(state.is_some() && !busy);
    }
}
impl Drop for Tray {
    fn drop(&mut self) {
        TrayIconEvent::set_event_handler(None::<fn(TrayIconEvent)>);
        MenuEvent::set_event_handler(None::<fn(MenuEvent)>);
    }
}

/// The macOS menu bar. Without one of its own, an app's menu is named after its executable file,
/// which would read "Quit huda-desktop". The About panel says what Huda is. Other systems have no
/// application menu bar, so there is nothing to build there.
pub struct AppMenu {
    #[cfg(target_os = "macos")]
    _menu: Menu,
}
impl AppMenu {
    /// Call only from App::logic, once the native event loop is running.
    pub fn create() -> Result<Option<Self>> {
        #[cfg(target_os = "macos")]
        {
            use tray_icon::menu::{accelerator::Accelerator, AboutMetadata, Submenu};
            let failed = |_| Error::new("MENU_UNAVAILABLE", "Cannot create the application menu");
            let about = AboutMetadata {
                name: Some(huda::APP_NAME.to_owned()),
                version: Some(huda::VERSION.to_owned()),
                short_version: Some(huda::VERSION.to_owned()),
                copyright: Some(huda::COPYRIGHT.to_owned()),
                credits: Some(huda::DESCRIPTION.to_owned()),
                ..Default::default()
            };
            let settings = MenuItem::with_id(
                "settings",
                "Settings…",
                true,
                "CmdOrCtrl+Comma".parse::<Accelerator>().ok(),
            );
            let application = Submenu::with_items(
                huda::APP_NAME,
                true,
                &[
                    &PredefinedMenuItem::about(Some("About Huda"), Some(about)),
                    &PredefinedMenuItem::separator(),
                    &settings,
                    &PredefinedMenuItem::separator(),
                    &PredefinedMenuItem::services(None),
                    &PredefinedMenuItem::separator(),
                    &PredefinedMenuItem::hide(Some("Hide Huda")),
                    &PredefinedMenuItem::hide_others(None),
                    &PredefinedMenuItem::show_all(None),
                    &PredefinedMenuItem::separator(),
                    &PredefinedMenuItem::quit(Some("Quit Huda")),
                ],
            )
            .map_err(failed)?;
            let menu = Menu::with_items(&[&application]).map_err(failed)?;
            menu.init_for_nsapp();
            Ok(Some(Self { _menu: menu }))
        }
        #[cfg(not(target_os = "macos"))]
        {
            Ok(None)
        }
    }
}

pub fn show(context: &egui::Context) {
    context.send_viewport_cmd(egui::ViewportCommand::Visible(true));
    context.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
    context.send_viewport_cmd(egui::ViewportCommand::Focus);
}
pub fn background(context: &egui::Context) {
    // A Linux SNI service may exist without a shell actually showing it. Never make
    // the only window unreachable; retain a taskbar entry even on GNOME without a tray host.
    #[cfg(target_os = "linux")]
    context.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
    #[cfg(not(target_os = "linux"))]
    context.send_viewport_cmd(egui::ViewportCommand::Visible(false));
}
