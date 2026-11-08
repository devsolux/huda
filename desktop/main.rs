#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
//! Native client: no SQLite connection, publication policy, model, or provider logic in the UI.
mod clients;
mod graph;
mod results;
mod shell;
mod startup;
mod theme;
mod widgets;

use clap::Parser;
use eframe::egui;
use huda::runtime::{
    ipc::{Client, Credential},
    Profile,
};
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tab {
    Space,
    Ask,
    Editor,
    Sources,
    Integrations,
    Clients,
    Diagnostics,
    Settings,
}
impl Tab {
    /// The sidebar: two levels at most, and nothing critical at its bottom.
    const GROUPS: [(&'static str, &'static [Tab]); 3] = [
        ("Knowledge", &[Tab::Space, Tab::Ask, Tab::Editor]),
        (
            "Connections",
            &[Tab::Sources, Tab::Integrations, Tab::Clients],
        ),
        ("System", &[Tab::Diagnostics, Tab::Settings]),
    ];
    /// In sidebar order, which Command and a digit follow.
    const ORDER: [Tab; 8] = [
        Tab::Space,
        Tab::Ask,
        Tab::Editor,
        Tab::Sources,
        Tab::Integrations,
        Tab::Clients,
        Tab::Diagnostics,
        Tab::Settings,
    ];
    const fn title(self) -> &'static str {
        match self {
            Tab::Space => "Relation Space",
            Tab::Ask => "Ask Huda",
            Tab::Editor => "Editor",
            Tab::Sources => "Sources",
            Tab::Integrations => "Integrations",
            Tab::Clients => "MCP Clients",
            Tab::Diagnostics => "Diagnostics",
            Tab::Settings => "Settings",
        }
    }
    const fn subtitle(self) -> &'static str {
        match self {
            Tab::Space => "Typed relations and the assertions behind them.",
            Tab::Ask => "Answers from the relation context you are authorized to read.",
            Tab::Editor => "Run any documented operation with its JSON input.",
            Tab::Sources => "Admit sources and decide what each provider may do.",
            Tab::Integrations => "Explicit settings for what Huda connects to.",
            Tab::Clients => "Connect the AI clients on this computer to this profile.",
            Tab::Diagnostics => "How the Core and what it connects to are doing.",
            Tab::Settings => "How Huda starts and keeps running on this computer.",
        }
    }
    const fn icon(self) -> widgets::Icon {
        match self {
            Tab::Space => widgets::Icon::Relation,
            Tab::Ask => widgets::Icon::Ask,
            Tab::Editor => widgets::Icon::Editor,
            Tab::Sources => widgets::Icon::Sources,
            Tab::Integrations => widgets::Icon::Integrations,
            Tab::Clients => widgets::Icon::Clients,
            Tab::Diagnostics => widgets::Icon::Diagnostics,
            Tab::Settings => widgets::Icon::Settings,
        }
    }
}
/// Below this width the sidebar keeps only its icons, so the content keeps its room.
const COMPACT_WIDTH: f32 = 760.0;
/// A window with its title bar overlaid leaves this much for the window controls.
const TITLE_BAR: f32 = if cfg!(target_os = "macos") {
    30.0
} else {
    10.0
};
struct Desktop {
    client: Client,
    clients: clients::Panel,
    tab: Tab,
    operation: String,
    draft: String,
    question: String,
    language: String,
    expansion: String,
    results: results::Results,
    pending: Option<mpsc::Receiver<Answer>>,
    cancellation: Arc<AtomicBool>,
    confirm_mutation: bool,
    /// The setup step the helper is running; requests, unlike setup, can be cancelled.
    pending_setup: Option<Setup>,
    /// The pending request adds an entity's relations to the answer on screen.
    pending_expansion: bool,
    /// A request found this profile's credential missing from the OS secret store, or no longer
    /// accepted, so the profile needs Restore Access rather than a reason with no way forward.
    needs_restore: bool,
    tray: Option<shell::Tray>,
    /// Kept for the process: macOS drops a menu bar whose owner is gone.
    _app_menu: Option<shell::AppMenu>,
    shell_started: bool,
    background_requested: bool,
    keep_running: bool,
    quitting: bool,
    startup_state: Option<bool>,
    startup_pending: Option<mpsc::Receiver<huda::error::Result<bool>>>,
    shell_status: String,
    /// A relaunch marker that cannot be read is reported once, not every second.
    activation_failed: bool,
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
            clients: clients::Panel::new(profile.clone(), context.clone()),
            client: Client::new(profile, "owner", true),
            tab: Tab::Space, operation: "explore_relations".into(), draft: "{}".into(),
            question: String::new(), language: "en".into(), expansion: "off".into(),
            results: results::Results::new(format!("{} {} is a desktop source candidate. No source scans, model downloads or provider requests occur on launch.", huda::APP_NAME, huda::VERSION)),
            pending: None, cancellation: Arc::new(AtomicBool::new(false)), confirm_mutation: false, pending_setup: None, pending_expansion: false,
            needs_restore: false, tray: None, _app_menu: None, shell_started: false, background_requested: background,
            keep_running: background, quitting: false, startup_state: None,
            startup_pending: None, shell_status: "Login registration has not been checked".into(),
            activation_failed: false, context,
            _instance: instance,
        }
    }
    /// Nothing in the Core can answer before setup, so a request would only fail with a reason
    /// about the secret store, which is not where the problem is.
    fn setup(&self) -> Option<Setup> {
        if !self.client.profile.database().exists() {
            Some(Setup::Initialize)
        } else if self.needs_restore {
            Some(Setup::Restore)
        } else {
            None
        }
    }
    fn needs_setup(&self) -> bool {
        self.setup().is_some()
    }
    fn submit(&mut self, operation: &str, input: Value) {
        if self.pending.is_some() {
            return;
        }
        if !self.client.profile.database().exists() {
            self.results.attention(
                "This profile is not set up yet, so nothing was sent. Choose Initialize Profile above.",
            );
            return;
        }
        self.pending_setup = None;
        self.pending_expansion = false;
        let (sender, receiver) = mpsc::sync_channel(1);
        let client = self.client.clone();
        let operation = operation.to_owned();
        self.cancellation = Arc::new(AtomicBool::new(false));
        let cancel = self.cancellation.clone();
        let context = self.context.clone();
        std::thread::spawn(move || {
            let _ = sender.send(request(&client, &operation, input, cancel));
            context.request_repaint();
        });
        self.pending = Some(receiver);
        self.confirm_mutation = false;
    }
    fn poll(&mut self) {
        if let Some(receiver) = &self.pending {
            match receiver.try_recv() {
                Ok(answer) => {
                    self.answered(answer);
                    self.pending = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.results.message(
                        "Client worker stopped; no automatic retry or mutation was attempted.",
                    );
                    self.pending = None;
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
    }
}
impl Desktop {
    /// Adds an entity's relations to the graph on screen.
    fn expand(&mut self, expand: &results::Expand) {
        if self.pending.is_some() {
            return;
        }
        self.submit("explore_relations", expand.request());
        self.pending_expansion = self.pending.is_some();
    }
    fn answered(&mut self, answer: Answer) {
        let expansion = std::mem::take(&mut self.pending_expansion);
        match (self.pending_setup.take(), answer) {
            (Some(_), Answer::Done(Ok(_))) => {
                self.needs_restore = false;
                // The helper issued new credentials; read them, not the ones read before.
                self.client.forget_credential();
                self.results.message(
                    "This profile is ready. Choose Explore Relations to see what it holds.",
                );
            }
            (Some(_), Answer::Done(error)) => {
                // An initialization that failed after creating the database is finished by
                // Restore Access, so that is what the profile offers next.
                self.needs_restore = self.client.profile.database().exists();
                self.results.set(error);
            }
            (_, Answer::Lost) => {
                self.needs_restore = true;
                self.results.attention(
                    "This profile's credential is missing from the OS secret store, or the profile no longer accepts it. Choose Restore Access above.",
                );
            }
            (None, Answer::Done(result)) => {
                self.needs_restore = false;
                if expansion {
                    self.results.merge(result);
                } else {
                    self.results.set(result);
                }
            }
        }
    }
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
    fn run_setup(&mut self, setup: Setup) {
        if self.pending.is_some() {
            return;
        }
        // Setup can display an OS-secret-store prompt; it is not a cancellable IPC request.
        self.pending_setup = Some(setup);
        let profile = self.client.profile.clone();
        let context = self.context.clone();
        let (sender, receiver) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            // Setup delegates to the fixed helper. The GUI never opens SQLite.
            let result = (|| -> huda::error::Result<Value> {
                let output = huda::runtime::helper::command()?
                    .arg("--profile")
                    .arg(profile.directory())
                    .args(setup.arguments())
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::piped())
                    .output()?;
                if output.status.success() {
                    Ok(json!({"ready": true}))
                } else {
                    Err(helper_error(&output.stderr, setup))
                }
            })();
            let _ = sender.send(Answer::Done(result));
            context.request_repaint();
        });
        self.pending = Some(receiver);
    }
}
/// What a request found.
enum Answer {
    /// What the Core answered, or why it could not be asked. A secret store that is locked or
    /// failing ends here: its own reason says what to do.
    Done(huda::error::Result<Value>),
    /// The credential is missing from the OS secret store, or the profile no longer accepts it.
    /// Only Restore Access fixes that.
    Lost,
}
/// Sends one request, reading the credential first, so a missing one is told apart from a store
/// that refuses access without asking the store twice.
fn request(client: &Client, operation: &str, input: Value, cancel: Arc<AtomicBool>) -> Answer {
    // A cancelled request never reads secrets; the client reports the cancellation.
    if !cancel.load(Ordering::Acquire) {
        match client.credential() {
            Ok(Credential::Stored) => {}
            Ok(Credential::Missing) => return Answer::Lost,
            Err(error) => return Answer::Done(Err(error)),
        }
    }
    match client.invoke_cancelled(operation, input, cancel) {
        // The Core refused the stored credential, even after the client read it again.
        Err(error) if error.code == "AUTH_REQUIRED" => Answer::Lost,
        result => Answer::Done(result),
    }
}
/// The one setup step a profile can need. The helper runs it, never the GUI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Setup {
    /// No database yet: create the profile and its credentials.
    Initialize,
    /// A credential is missing or no longer accepted: issue new ones where needed.
    Restore,
}
impl Setup {
    fn arguments(self) -> &'static [&'static str] {
        match self {
            Self::Initialize => &["init"],
            Self::Restore => &["credentials", "restore"],
        }
    }
    /// The same step in a terminal, which shows a reason the helper could not give.
    fn command(self) -> &'static str {
        match self {
            Self::Initialize => "huda init",
            Self::Restore => "huda credentials restore",
        }
    }
    fn title(self) -> &'static str {
        match self {
            Self::Initialize => "Set up this profile",
            Self::Restore => "Restore access to this profile",
        }
    }
    fn detail(self) -> &'static str {
        match self {
            Self::Initialize => "Creates a private local profile and its credentials in the OS secret store. Nothing is sent anywhere.",
            Self::Restore => "Issues a new credential where one is missing or no longer accepted, and keeps every one that still works. Relations and history do not change.",
        }
    }
    fn action(self) -> &'static str {
        match self {
            Self::Initialize => "Initialize Profile",
            Self::Restore => "Restore Access",
        }
    }
    fn progress(self) -> &'static str {
        match self {
            Self::Initialize => "Initializing profile; complete any OS keychain prompt",
            Self::Restore => "Restoring access; complete any OS keychain prompt",
        }
    }
}
/// The reason the helper gave. It prints one redacted JSON error, which names no path, token or
/// payload, so it can be shown as it is. Anything else gets a reason that still says what to do.
fn helper_error(stderr: &[u8], setup: Setup) -> huda::error::Error {
    const MAX: usize = 4096;
    let text = String::from_utf8_lossy(&stderr[..stderr.len().min(MAX)]);
    text.lines()
        .rev()
        .find_map(|line| {
            let value: Value = serde_json::from_str(line).ok()?;
            serde_json::from_value(value.get("error")?.clone()).ok()
        })
        .unwrap_or_else(|| {
            huda::error::Error::new(
                "SETUP_FAILED",
                &format!(
                    "{} did not complete. Run {} in a terminal to see why; do not delete an existing profile",
                    setup.action(),
                    setup.command()
                ),
            )
        })
}
impl eframe::App for Desktop {
    fn logic(&mut self, context: &egui::Context, _: &mut eframe::Frame) {
        self.poll();
        self.clients.poll();
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
                    self.results.message(error.to_string());
                    shell::show(context);
                }
            }
            // Only after the event loop runs: it replaces the menu macOS would otherwise name
            // after the executable file.
            match shell::AppMenu::create() {
                Ok(menu) => self._app_menu = menu,
                Err(error) => self.shell_status = error.to_string(),
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
                shell::Action::Settings => {
                    self.tab = Tab::Settings;
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
        match self.client.profile.take_desktop_activation() {
            Ok(true) => shell::show(context),
            Ok(false) => {}
            Err(error) if !self.activation_failed => {
                self.activation_failed = true;
                self.shell_status = format!("Relaunching cannot show this window: {error}");
            }
            Err(_) => {}
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
        self.body(ui);
    }
}
impl Desktop {
    fn body(&mut self, ui: &mut egui::Ui) {
        let context = ui.ctx().clone();
        self.shortcuts(&context);
        self.sidebar_ui(ui);
        let palette = theme::palette(&context);
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(palette.window)
                    .inner_margin(egui::Margin {
                        left: 24,
                        right: 24,
                        top: TITLE_BAR as i8 + 6,
                        bottom: 16,
                    }),
            )
            .show(ui, |ui| self.detail_ui(ui));
        widgets::focus_ring(&context);
    }
    fn shortcuts(&mut self, context: &egui::Context) {
        use egui::{Key, Modifiers};
        const DIGITS: [Key; 8] = [
            Key::Num1,
            Key::Num2,
            Key::Num3,
            Key::Num4,
            Key::Num5,
            Key::Num6,
            Key::Num7,
            Key::Num8,
        ];
        context.input_mut(|input| {
            for (key, tab) in DIGITS.into_iter().zip(Tab::ORDER) {
                if input.consume_key(Modifiers::COMMAND, key) {
                    self.tab = tab;
                }
            }
            if input.consume_key(Modifiers::COMMAND, Key::Comma) {
                self.tab = Tab::Settings;
            }
        });
    }
    fn sidebar_ui(&mut self, ui: &mut egui::Ui) {
        let palette = theme::palette(ui.ctx());
        let compact = ui.available_width() < COMPACT_WIDTH;
        egui::Panel::left("sidebar")
            .resizable(false)
            .exact_size(if compact { 56.0 } else { 212.0 })
            .frame(
                egui::Frame::new()
                    .fill(palette.sidebar)
                    .inner_margin(egui::Margin {
                        left: 10,
                        right: 10,
                        top: TITLE_BAR as i8 + 6,
                        bottom: 10,
                    }),
            )
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.ctx().accesskit_node_builder(ui.unique_id(), |node| {
                    node.set_role(egui::accesskit::Role::Navigation);
                    node.set_label("Areas".to_owned());
                });
                ui.horizontal(|ui| {
                    widgets::monogram(ui, 20.0);
                    if !compact {
                        ui.label(
                            egui::RichText::new("Huda")
                                .font(egui::FontId::new(15.0, theme::semibold())),
                        );
                    }
                });
                ui.add_space(10.0);
                for (title, tabs) in Tab::GROUPS {
                    if !compact {
                        ui.add_space(6.0);
                        ui.label(
                            egui::RichText::new(title)
                                .font(theme::section_title())
                                .color(palette.secondary),
                        );
                        ui.add_space(2.0);
                    } else {
                        ui.add_space(8.0);
                    }
                    for tab in tabs {
                        let selected = self.tab == *tab;
                        if widgets::sidebar_row(ui, tab.icon(), tab.title(), selected, compact)
                            .clicked()
                        {
                            self.tab = *tab;
                        }
                    }
                }
            });
    }
    fn detail_ui(&mut self, ui: &mut egui::Ui) {
        if self.tab == Tab::Clients {
            self.clients.open();
        }
        widgets::page_header(ui, self.tab.title(), self.tab.subtitle());
        if self.pending.is_some() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(
                    self.pending_setup
                        .map_or("Request in progress", Setup::progress),
                );
                if ui
                    .add_enabled(
                        self.pending_setup.is_none(),
                        egui::Button::new("Cancel Request"),
                    )
                    .clicked()
                {
                    self.cancellation.store(true, Ordering::Release);
                }
            });
            ui.add_space(6.0);
        }
        let tab = self.tab;
        // Every area that asks the Core offers setup first, not only Relation Space.
        if self.needs_setup() && !matches!(tab, Tab::Clients | Tab::Settings) {
            ui.add_enabled_ui(self.pending.is_none(), |ui| self.setup_ui(ui));
        }
        ui.add_enabled_ui(self.pending.is_none(), |ui| match tab {
            Tab::Space => self.space_ui(ui),
            Tab::Ask => self.ask_ui(ui),
            Tab::Editor | Tab::Sources | Tab::Integrations => self.operation_ui(ui),
            Tab::Clients => self.clients.ui(ui),
            Tab::Diagnostics => self.diagnostics_ui(ui),
            Tab::Settings => self.settings_ui(ui),
        });
        // These two ask the Core nothing, so they have no answer to show.
        if !matches!(self.tab, Tab::Clients | Tab::Settings) {
            ui.add_space(4.0);
            if let Some(expand) = self.results.ui(ui) {
                self.expand(&expand);
            }
        }
    }
    fn setup_ui(&mut self, ui: &mut egui::Ui) {
        let Some(setup) = self.setup() else {
            return;
        };
        widgets::section(ui, None, |section| {
            section.row(|ui| {
                widgets::row_with_trailing(
                    ui,
                    |ui| {
                        widgets::row_title(ui, setup.title());
                        widgets::row_detail(ui, setup.detail());
                    },
                    |ui| {
                        if widgets::primary_button(ui, setup.action()).clicked() {
                            self.run_setup(setup);
                        }
                    },
                );
            });
        });
    }
    fn space_ui(&mut self, ui: &mut egui::Ui) {
        // One primary action: before setup, that is setup.
        let needs_setup = self.needs_setup();
        ui.horizontal(|ui| {
            let explore = if needs_setup {
                ui.button("Explore Relations")
            } else {
                widgets::primary_button(ui, "Explore Relations")
            };
            if explore.clicked() {
                self.submit("explore_relations", json!({}));
            }
            if ui.button("Registered Types").clicked() {
                self.submit("list_relation_types", json!({}));
            }
            if ui.button("Review Proposals").clicked() {
                self.submit("list_proposals", json!({}));
            }
        });
        widgets::row_detail(
            ui,
            "Relations are primary · Evidence is optional · Context never grants permission",
        );
    }
    fn ask_ui(&mut self, ui: &mut egui::Ui) {
        let question = ui.add(
            egui::TextEdit::multiline(&mut self.question)
                .hint_text("What does Project A depend on in production?")
                .desired_rows(3)
                .desired_width(f32::INFINITY),
        );
        widgets::labelled(&question, "Question");
        ui.horizontal(|ui| {
            let caption = ui.label("Language");
            ui.add(egui::TextEdit::singleline(&mut self.language).desired_width(48.0))
                .labelled_by(caption.id);
            let expansion = egui::ComboBox::from_id_salt("expansion")
                .selected_text(expansion_label(&self.expansion))
                .show_ui(ui, |ui| {
                    for value in ["off", "suggest", "auto"] {
                        ui.selectable_value(
                            &mut self.expansion,
                            value.into(),
                            expansion_label(value),
                        );
                    }
                });
            widgets::labelled(&expansion.response, "Research");
            if widgets::primary_button(ui, "Ask Huda").clicked() {
                self.submit(
                    "ask_huda",
                    json!({"question": self.question, "language": self.language, "expansion_mode": self.expansion}),
                );
            }
        });
        widgets::row_detail(
            ui,
            "No model: structured graph queries still work. Async expansion returns a job ID; inspect the job and re-ask with expansion off after completion.",
        );
    }
    fn operation_ui(&mut self, ui: &mut egui::Ui) {
        widgets::row_detail(
            ui,
            "Advanced JSON forms use the actual handler schemas (`huda contracts`). Refresh status to obtain the current policy epoch. Publication validates roles, scope, versions and consent in Core.",
        );
        if self.tab == Tab::Sources {
            widgets::row_detail(
                ui,
                "Provider permissions are independent: may_fetch, auto_register_provider_types, auto_admit_structured, auto_admit_semantic. Public search terms require explicit approval. Raw questions are never automatically approved for disclosure.",
            );
        }
        ui.add_space(4.0);
        let operation = egui::ComboBox::from_id_salt("operation")
            .width(280.0)
            .selected_text(self.operation.as_str())
            .show_ui(ui, |ui| {
                for operation in huda::api::contracts::OPERATIONS {
                    ui.selectable_value(&mut self.operation, operation.name.into(), operation.name);
                }
            });
        widgets::labelled(&operation.response, "Operation");
        egui::ScrollArea::vertical()
            .id_salt("editor")
            .max_height(180.0)
            .show(ui, |ui| {
                let input = ui.add(
                    egui::TextEdit::multiline(&mut self.draft)
                        .code_editor()
                        .desired_rows(7)
                        .desired_width(f32::INFINITY),
                );
                widgets::labelled(&input, "Operation input as JSON");
            });
        let write = huda::api::contracts::OPERATIONS
            .iter()
            .any(|op| op.name == self.operation && !op.read_only);
        if write {
            ui.checkbox(
                &mut self.confirm_mutation,
                "I explicitly authorize this operation with the displayed JSON and current policy/version",
            );
        }
        ui.add_enabled_ui(!write || self.confirm_mutation, |ui| {
            if widgets::primary_button(ui, "Run Operation").clicked() {
                match serde_json::from_str(&self.draft) {
                    Ok(value) => {
                        let operation = self.operation.clone();
                        self.submit(&operation, value);
                    }
                    Err(_) => self
                        .results
                        .message("The request is not valid JSON. No operation was sent."),
                }
            }
        });
    }
    fn diagnostics_ui(&mut self, ui: &mut egui::Ui) {
        let mut asked = None;
        widgets::section(ui, None, |section| {
            for (title, detail, label, operation) in [
                ("Core", "Health and current epochs.", "Check Core", "status"),
                (
                    "Providers",
                    "What each knowledge provider may do.",
                    "Check Providers",
                    "get_knowledge_providers",
                ),
                (
                    "Bridge",
                    "Explicitly configured MCP reads.",
                    "Check Bridge",
                    "bridge_status",
                ),
            ] {
                section.row(|ui| {
                    widgets::row_with_trailing(
                        ui,
                        |ui| {
                            widgets::row_title(ui, title);
                            widgets::row_detail(ui, detail);
                        },
                        |ui| {
                            if ui.button(label).clicked() {
                                asked = Some(operation);
                            }
                        },
                    );
                });
            }
        });
        if let Some(operation) = asked {
            self.submit(operation, json!({}));
        }
        widgets::row_detail(
            ui,
            "Backup uses the shared `backup` handler. No wipe, destructive repair, restore-overwrite or uninstall erasure is offered.",
        );
    }
    fn settings_ui(&mut self, ui: &mut egui::Ui) {
        let mut startup = None;
        let mut check = false;
        let mut quit = false;
        widgets::section(ui, Some("Startup"), |section| {
            section.row(|ui| {
                widgets::row_with_trailing(
                    ui,
                    |ui| {
                        widgets::row_title(ui, "Launch at Login");
                        widgets::row_detail(ui, &self.shell_status);
                    },
                    |ui| {
                        check = ui
                            .add_enabled(
                                self.startup_pending.is_none(),
                                egui::Button::new("Check Again"),
                            )
                            .clicked();
                        let mut enabled = self.startup_state.unwrap_or(false);
                        let ready = self.startup_state.is_some() && self.startup_pending.is_none();
                        if ui
                            .add_enabled(
                                ready,
                                widgets::Switch::new(&mut enabled, "Launch at Login"),
                            )
                            .changed()
                        {
                            startup = Some(enabled);
                        }
                    },
                );
            });
            section.row(|ui| {
                widgets::row_with_trailing(
                    ui,
                    |ui| {
                        widgets::row_title(ui, "Keep Running After Close");
                        widgets::row_detail(ui, "For this session. Huda stays in the tray.");
                    },
                    |ui| {
                        ui.add_enabled(
                            self.tray.is_some(),
                            widgets::Switch::new(
                                &mut self.keep_running,
                                "Keep Running After Close",
                            ),
                        );
                    },
                );
            });
        });
        widgets::section(ui, Some("This Profile"), |section| {
            section.row(|ui| {
                widgets::row_title(
                    ui,
                    if self.client.profile.database().exists() {
                        "Initialized"
                    } else {
                        "Not initialized"
                    },
                );
                widgets::row_detail(ui, &self.client.profile.directory().display().to_string());
            });
        });
        widgets::section(ui, Some("About"), |section| {
            section.row(|ui| {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    widgets::monogram(ui, 36.0);
                    ui.vertical(|ui| {
                        ui.label(
                            egui::RichText::new(huda::APP_NAME)
                                .font(egui::FontId::new(15.0, theme::semibold())),
                        );
                        ui.label(huda::DESCRIPTION);
                        widgets::row_detail(
                            ui,
                            &format!("Version {} · {}", huda::VERSION, huda::APP_ID),
                        );
                        widgets::row_detail(ui, &format!("MIT License · {}", huda::COPYRIGHT));
                    });
                });
            });
            section.row(|ui| {
                widgets::row_with_trailing(
                    ui,
                    |ui| {
                        widgets::row_title(ui, "Quit Huda");
                        widgets::row_detail(
                            ui,
                            "Quitting the desktop never stops a Core that serves another client.",
                        );
                    },
                    |ui| quit = ui.button("Quit Huda").clicked(),
                );
            });
        });
        if let Some(enabled) = startup {
            self.startup_request(Some(enabled));
        }
        if check {
            self.startup_request(None);
        }
        if quit {
            self.quitting = true;
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}
fn expansion_label(value: &str) -> &'static str {
    match value {
        "suggest" => "Suggest research",
        "auto" => "Expand automatically under existing consent",
        _ => "Graph only",
    }
}
#[derive(Parser)]
#[command(name = "Huda", version = huda::VERSION, about = huda::DESCRIPTION)]
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
            .with_min_inner_size([640.0, 480.0])
            // On macOS the sidebar runs under the window controls, as in the system's own apps.
            .with_fullsize_content_view(true)
            .with_titlebar_shown(false)
            .with_title_shown(false)
            .with_icon(icon),
        ..Default::default()
    };
    GUI_STARTED.store(true, Ordering::Release);
    eframe::run_native(
        huda::APP_NAME,
        options,
        Box::new(move |context| {
            theme::apply(&context.egui_ctx);
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
/// Set once the desktop window starts; a failure before it is shown in a window of its own.
static GUI_STARTED: AtomicBool = AtomicBool::new(false);
/// GUI launches (Finder, login items, the Windows subsystem) have no visible stderr. If the
/// window system itself cannot start, only stderr and the exit code remain.
fn show_startup_error(message: String) {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_app_id(huda::APP_ID)
            .with_title(huda::APP_NAME)
            .with_inner_size([520.0, 200.0]),
        ..Default::default()
    };
    let _ = eframe::run_ui_native(huda::APP_NAME, options, move |ui, _| {
        ui.heading("Huda could not start");
        ui.label(&message);
        if ui.button("Quit Huda").clicked() {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
    });
}
fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Huda: {error}");
            if !GUI_STARTED.load(Ordering::Acquire) {
                show_startup_error(error.to_string());
            }
            std::process::ExitCode::FAILURE
        }
    }
}
#[cfg(test)]
mod accessibility_tests;
#[cfg(test)]
mod setup_tests;
