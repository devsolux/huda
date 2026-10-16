#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
//! Native client: no SQLite connection, publication policy, model, or provider logic in the UI.
mod shell;
mod startup;

use clap::Parser;
use eframe::egui;
use huda::runtime::{ipc::Client, Profile};
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Space,
    Editor,
    Ask,
    Sources,
    Integrations,
    Diagnostics,
}
struct Desktop {
    client: Client,
    tab: Tab,
    operation: String,
    draft: String,
    question: String,
    language: String,
    expansion: String,
    output: String,
    pending: Option<mpsc::Receiver<huda::error::Result<Value>>>,
    cancellation: Arc<AtomicBool>,
    confirm_mutation: bool,
    pending_cancellable: bool,
    /// A request reported AUTH_REQUIRED: the database exists, but initialization never finished.
    needs_init: bool,
    tray: Option<shell::Tray>,
    shell_started: bool,
    background_requested: bool,
    keep_running: bool,
    quitting: bool,
    startup_state: Option<bool>,
    startup_pending: Option<mpsc::Receiver<huda::error::Result<bool>>>,
    shell_status: String,
    context: egui::Context,
    /// Held for the process lifetime so repeated launches do not create duplicate trays.
    _instance: huda::runtime::profile::Lease,
}
impl Desktop {
    fn new(
        profile: Profile,
        background: bool,
        context: egui::Context,
        instance: huda::runtime::profile::Lease,
    ) -> Self {
        Self {
            client: Client { profile, actor: "owner".into(), start_on_demand: true },
            tab: Tab::Space, operation: "explore_relations".into(), draft: "{}".into(),
            question: String::new(), language: "en".into(), expansion: "off".into(),
            output: format!("{} {} — desktop source candidate.\nUse Initialize profile once, then inspect the relation space.\nNo source scans, model downloads, or provider requests occur on launch.", huda::APP_NAME, huda::VERSION),
            pending: None, cancellation: Arc::new(AtomicBool::new(false)), confirm_mutation: false, pending_cancellable: true,
            needs_init: false, tray: None, shell_started: false, background_requested: background,
            keep_running: background, quitting: false, startup_state: None,
            startup_pending: None, shell_status: "Login registration has not been checked".into(), context,
            _instance: instance,
        }
    }
    fn submit(&mut self, operation: &str, input: Value) {
        if self.pending.is_some() {
            return;
        }
        self.pending_cancellable = true;
        let (sender, receiver) = mpsc::sync_channel(1);
        let client = self.client.clone();
        let operation = operation.to_owned();
        self.cancellation = Arc::new(AtomicBool::new(false));
        let cancel = self.cancellation.clone();
        let context = self.context.clone();
        std::thread::spawn(move || {
            let _ = sender.send(client.invoke_cancelled(&operation, input, cancel));
            context.request_repaint();
        });
        self.pending = Some(receiver);
        self.confirm_mutation = false;
    }
    fn poll(&mut self) {
        if let Some(receiver) = &self.pending {
            match receiver.try_recv() {
                Ok(result) => {
                    self.needs_init =
                        matches!(&result, Err(error) if error.code == "AUTH_REQUIRED");
                    self.output = match result {
                        Ok(value) => serde_json::to_string_pretty(&value)
                            .unwrap_or_else(|_| "Response could not be displayed".into()),
                        Err(error) => format!("{}: {}", error.code, error.message),
                    };
                    self.pending = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.output =
                        "Client worker stopped; no automatic retry or mutation was attempted."
                            .into();
                    self.pending = None;
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
    }
}
impl Desktop {
    fn startup_request(&mut self, requested: Option<bool>) {
        if self.startup_pending.is_some() {
            return;
        }
        let (sender, receiver) = mpsc::sync_channel(1);
        let context = self.context.clone();
        std::thread::spawn(move || {
            let _ = sender.send(startup::apply(requested));
            context.request_repaint();
        });
        self.startup_pending = Some(receiver);
        self.shell_status = "Checking this user's OS login registration…".into();
    }
    fn initialize_profile(&mut self) {
        if self.pending.is_some() {
            return;
        }
        // Initialization can display an OS-secret-store prompt; it is not a cancellable IPC request.
        self.pending_cancellable = false;
        let profile = self.client.profile.clone();
        let context = self.context.clone();
        let (sender, receiver) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            // Explicit bootstrap delegates to the fixed helper. The GUI never opens SQLite.
            let result = (|| -> huda::error::Result<Value> {
                let status = huda::runtime::helper::command()?
                    .arg("--profile")
                    .arg(profile.directory())
                    .arg("init")
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status()?;
                huda::error::require(status.success(), "INIT_FAILED",
                    "Initialization did not complete. Run huda init in a terminal for its redacted error; do not delete an existing profile")?;
                Ok(json!({"initialized": true, "next": "Explore relations or inspect Core health"}))
            })();
            let _ = sender.send(result);
            context.request_repaint();
        });
        self.pending = Some(receiver);
    }
}
impl eframe::App for Desktop {
    fn logic(&mut self, context: &egui::Context, _: &mut eframe::Frame) {
        self.poll();
        if !self.shell_started {
            self.shell_started = true;
            match shell::Tray::create(context) {
                Ok(tray) => {
                    self.tray = Some(tray);
                    if self.background_requested {
                        shell::background(context);
                    }
                }
                Err(error) => {
                    self.output = error.to_string();
                    shell::show(context);
                }
            }
            self.startup_request(None); // Read only; never enables autostart on launch.
        }
        if let Some(receiver) = &self.startup_pending {
            match receiver.try_recv() {
                Ok(result) => {
                    match result {
                        Ok(state) => {
                            self.startup_state = Some(state);
                            self.shell_status = if state {
                                "Login launch is registered. The OS can still disable it."
                            } else {
                                "Launch at login is off."
                            }
                            .into();
                        }
                        Err(error) => {
                            self.startup_state = None;
                            self.shell_status = error.to_string();
                        }
                    }
                    self.startup_pending = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.startup_state = None;
                    self.startup_pending = None;
                    self.shell_status =
                        "Login-registration worker stopped; status is unknown".into();
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        let events = self
            .tray
            .as_ref()
            .map(|tray| tray.drain())
            .unwrap_or_default();
        for event in events {
            match event {
                shell::Action::Show => shell::show(context),
                shell::Action::Ask => {
                    self.tab = Tab::Ask;
                    shell::show(context);
                }
                shell::Action::Startup => {
                    if let Some(state) = self.startup_state {
                        self.startup_request(Some(!state));
                    }
                }
                shell::Action::Quit => {
                    self.quitting = true;
                    context.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
        if let Some(tray) = &self.tray {
            tray.startup_state(self.startup_state, self.startup_pending.is_some());
        }
        // A second launch cannot reach this process directly; it leaves an activation marker
        // in the profile directory and exits. Poll it slowly so a hidden window stays reachable.
        if self
            .client
            .profile
            .take_desktop_activation()
            .unwrap_or(false)
        {
            shell::show(context);
        }
        context.request_repaint_after(std::time::Duration::from_secs(1));
    }
    fn on_exit(&mut self, _: Option<&eframe::glow::Context>) {
        self.cancellation.store(true, Ordering::Release);
        let _ = self.tray.take(); // Release only the desktop's tray, not a shared Core.
    }
    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        let context = ui.ctx().clone();
        // Hidden eframe logic retains the last shown frame's input. Handle close only in
        // a fresh UI pass, otherwise a tray Show event could immediately hide it again.
        if !self.quitting
            && self.keep_running
            && self.tray.is_some()
            && context.input(|input| input.viewport().close_requested())
        {
            context.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            shell::background(&context);
        }
        egui::CentralPanel::default().show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.heading("Huda");
                for (tab, label) in [(Tab::Space, "Relation Space"), (Tab::Editor, "Editor"), (Tab::Ask, "Ask Huda"), (Tab::Sources, "Sources"), (Tab::Integrations, "Integrations"), (Tab::Diagnostics, "Diagnostics")] {
                    if ui.selectable_label(self.tab == tab, label).clicked() { self.tab = tab; }
                }
            });
            ui.label("Relations are primary · Evidence is optional · Context never grants permission");
            ui.separator();
            ui.horizontal_wrapped(|ui| {
                let mut startup_enabled = self.startup_state.unwrap_or(false);
                if ui.add_enabled(self.startup_state.is_some() && self.startup_pending.is_none(),
                    egui::Checkbox::new(&mut startup_enabled, "Launch at login")).changed() {
                    self.startup_request(Some(startup_enabled));
                }
                if ui.add_enabled(self.startup_pending.is_none(), egui::Button::new("Refresh login status")).clicked() {
                    self.startup_request(None);
                }
                ui.add_enabled(self.tray.is_some(), egui::Checkbox::new(&mut self.keep_running, "Keep running after close (this session)"));
                if ui.button("Quit Huda").clicked() {
                    self.quitting = true; context.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.small(&self.shell_status);
            ui.separator();
            if self.pending.is_some() {
                ui.horizontal(|ui| {
                    ui.spinner(); ui.label(if self.pending_cancellable { "Request in progress" } else { "Initializing profile; complete any OS keychain prompt" });
                    if ui.add_enabled(self.pending_cancellable, egui::Button::new("Cancel request")).clicked() { self.cancellation.store(true, Ordering::Release); }
                });

            }
            ui.add_enabled_ui(self.pending.is_none(), |ui| {
                match self.tab {
                    Tab::Space => {
                        ui.heading("Explore typed relations and their assertions");
                        if self.needs_init || !self.client.profile.database().exists() {
                            ui.label("Initialize a private local profile and OS-store credentials. This enables no network access.");
                            if ui.button("Initialize profile").clicked() { self.initialize_profile(); }
                        }
                        ui.horizontal(|ui| {
                            if ui.button("Explore relations").clicked() { self.submit("explore_relations", json!({})); }
                            if ui.button("Registered types").clicked() { self.submit("list_relation_types", json!({})); }
                            if ui.button("Review proposals").clicked() { self.submit("list_proposals", json!({})); }
                        });
                    }
                    Tab::Ask => {
                        ui.heading("Ask from the authorized relation context");
                        ui.add(egui::TextEdit::multiline(&mut self.question).desired_rows(3).desired_width(f32::INFINITY));
                        ui.horizontal(|ui| {
                            ui.label("Language"); ui.text_edit_singleline(&mut self.language);
                            egui::ComboBox::from_id_salt("expansion").selected_text(&self.expansion).show_ui(ui, |ui| {
                                for (value, label) in [("off", "Graph only"), ("suggest", "Suggest research"), ("auto", "Expand automatically under existing consent")] {
                                    ui.selectable_value(&mut self.expansion, value.into(), label);
                                }
                            });
                            if ui.button("Ask Huda").clicked() {
                                self.submit("ask_huda", json!({"question": self.question, "language": self.language, "expansion_mode": self.expansion}));
                            }
                        });
                        ui.label("No model: structured graph queries still work. Async expansion returns a job ID; inspect the job and re-ask with expansion off after completion.");
                    }
                    Tab::Diagnostics => {
                        ui.horizontal(|ui| {
                            if ui.button("Core health / current epochs").clicked() { self.submit("status", json!({})); }
                            if ui.button("Provider status").clicked() { self.submit("get_knowledge_providers", json!({})); }
                            if ui.button("Bridge status").clicked() { self.submit("bridge_status", json!({})); }
                        });
                        ui.label("Backup uses the shared `backup` handler. No wipe, destructive repair, restore-overwrite, or uninstall erasure is offered.");
                    }
                    Tab::Editor | Tab::Sources | Tab::Integrations => {
                        ui.heading(match self.tab { Tab::Editor => "Typed relation editor", Tab::Sources => "Source admission and provider permissions", _ => "Explicit integration settings" });
                        ui.label("Advanced JSON forms use the actual handler schemas (`huda contracts`). Refresh status to obtain the current policy epoch. Publication validates roles, scope, versions and consent in Core.");
                        if self.tab == Tab::Sources {
                            ui.label("Provider permissions are independent: may_fetch, auto_register_provider_types, auto_admit_structured, auto_admit_semantic. Public search terms require explicit approval. Raw questions are never automatically approved for disclosure.");
                        }
                        egui::ComboBox::from_id_salt("operation").selected_text(&self.operation).show_ui(ui, |ui| {
                            for operation in huda::api::contracts::OPERATIONS {
                                ui.selectable_value(&mut self.operation, operation.name.into(), operation.name);
                            }
                        });
                        egui::ScrollArea::vertical().id_salt("editor").max_height(220.0).show(ui, |ui| {
                            ui.add(egui::TextEdit::multiline(&mut self.draft).code_editor().desired_rows(9).desired_width(f32::INFINITY));
                        });
                        let write = huda::api::contracts::OPERATIONS.iter().any(|op| op.name == self.operation && !op.read_only);
                        if write { ui.checkbox(&mut self.confirm_mutation, "I explicitly authorize this operation with the displayed JSON and current policy/version"); }
                        if ui.add_enabled(!write || self.confirm_mutation, egui::Button::new("Run operation")).clicked() {
                            match serde_json::from_str(&self.draft) {
                                Ok(value) => { let operation = self.operation.clone(); self.submit(&operation, value); }
                                Err(_) => self.output = "The request is not valid JSON. No operation was sent.".into(),
                            }
                        }
                    }
                }
            });
            ui.separator();
            ui.label("Authorized result — IDs, typed roles, origin, conditions, coverage, errors and optional evidence");
            egui::ScrollArea::both().id_salt("result").show(ui, |ui| {
                ui.add(egui::TextEdit::multiline(&mut self.output).code_editor().interactive(false).desired_rows(24).desired_width(f32::INFINITY));
            });
        });
    }
}
#[derive(Parser)]
#[command(name = "Huda", version = huda::VERSION, about = "Huda native relation-space desktop")]
struct Options {
    /// Login launches stay in the tray; Linux keeps a recoverable minimized window.
    #[arg(long)]
    background: bool,
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args = Options::parse();
    #[cfg(windows)]
    {
        let identifier: Vec<u16> = huda::APP_ID.encode_utf16().chain(Some(0)).collect();
        // SAFETY: identifier is terminated and lives through the OS call.
        let result = unsafe {
            windows_sys::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID(
                identifier.as_ptr(),
            )
        };
        if result < 0 {
            return Err("Windows application identity could not be set".into());
        }
    }
    let profile = Profile::default_location()?;
    // One desktop per profile. This lease is desktop-instance coordination only; it is not
    // the Core writer lease and it never starts, stops or implies acquisition activity.
    let instance = match profile.desktop_lease() {
        Ok(lease) => lease,
        Err(error) if error.code == "DESKTOP_RUNNING" => {
            if !args.background {
                // A manual relaunch asks the running desktop to show its window instead of
                // opening a second window and tray. Login relaunches stay silent.
                profile.request_desktop_activation()?;
            }
            eprintln!("Huda: {}; activating it instead", error.message);
            return Ok(());
        }
        Err(error) => return Err(error.into()),
    };
    // A marker left for a previous instance that exited before consuming it must not show
    // this window, in particular not for a --background login start.
    let _ = profile.take_desktop_activation();
    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../assets/icons/huda-256.png"))?;
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_app_id(huda::APP_ID)
            .with_title(huda::APP_NAME)
            .with_inner_size([1080.0, 780.0])
            .with_icon(icon),
        ..Default::default()
    };
    eframe::run_native(
        huda::APP_NAME,
        options,
        Box::new(move |context| {
            Ok(Box::new(Desktop::new(
                profile,
                args.background,
                context.egui_ctx.clone(),
                instance,
            )))
        }),
    )?;
    Ok(())
}
fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Huda: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}
