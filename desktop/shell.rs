//! Native tray lifetime and window actions, owned only by the desktop/main thread.
use eframe::egui;
use huda::error::{Error, Result};
use std::sync::mpsc::{self, Receiver};
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder, TrayIconEvent};

pub enum Action {
    Show,
    Ask,
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
            .with_tooltip("Huda — local relation space")
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
