//! The MCP Clients tab. Everything here runs on a worker thread and only after someone opens the
//! tab, so a launch never reads another product's files.
use crate::widgets::{self, Tone};
use eframe::egui;
use huda::mcp::clients::{self, Activity, ClientId, ClientState, ClientStatus, Server};
use huda::runtime::Profile;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{SystemTime, UNIX_EPOCH};

const GROUPS: [&str; 4] = [
    "On this computer",
    "Not installed",
    "Connect by hand",
    "Web apps",
];

fn group(state: ClientState) -> usize {
    match state {
        ClientState::Available
        | ClientState::Configured
        | ClientState::NeedsRepair
        | ClientState::Blocked => 0,
        ClientState::NotInstalled => 1,
        ClientState::Manual => 2,
        ClientState::Hosted => 3,
    }
}

/// How a row's status reads. Only seeing the client call a tool earns the good tone.
fn tone(row: &ClientStatus) -> Tone {
    match row.state {
        ClientState::Blocked => Tone::Problem,
        ClientState::NeedsRepair => Tone::Attention,
        _ if is_warning(row) => Tone::Attention,
        ClientState::Configured if row.activity.used_ms.is_some() => Tone::Good,
        _ => Tone::Quiet,
    }
}

fn state_label(state: ClientState) -> &'static str {
    match state {
        ClientState::NotInstalled => "Not installed",
        ClientState::Available => "Ready",
        ClientState::Configured => "Configured",
        ClientState::NeedsRepair => "Needs repair",
        ClientState::Blocked => "Blocked",
        ClientState::Manual => "Paste by hand",
        ClientState::Hosted => "Cannot connect",
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as u64)
}

fn ago(then_ms: u64, now_ms: u64) -> String {
    let unit = |count: u64, name: &str| {
        let plural = if count == 1 { "" } else { "s" };
        format!("{count} {name}{plural} ago")
    };
    match now_ms.saturating_sub(then_ms) / 60_000 {
        0 => "just now".to_owned(),
        minutes @ 1..=59 => unit(minutes, "minute"),
        minutes @ 60..=1439 => unit(minutes / 60, "hour"),
        minutes => unit(minutes / 1440, "day"),
    }
}

/// What was seen of a client, which the file alone cannot say. Empty when nothing was.
fn evidence(activity: &Activity, now_ms: u64) -> String {
    match (activity.is_refused(), activity.refused_ms, activity.used_ms) {
        (true, Some(refused), _) => format!(
            "Refused {}: it does not speak MCP 2026-07-28, the only version Huda serves.",
            ago(refused, now_ms)
        ),
        (_, _, Some(used)) => format!("Last used {}.", ago(used, now_ms)),
        _ => String::new(),
    }
}

/// The small line under a client's name.
fn detail(row: &ClientStatus, now_ms: u64) -> String {
    let seen = evidence(&row.activity, now_ms);
    match row.state {
        ClientState::Configured if seen.is_empty() => {
            "Restart the client to finish. It has not used Huda yet.".to_owned()
        }
        ClientState::Configured | ClientState::Manual if !seen.is_empty() => seen,
        ClientState::NeedsRepair => {
            "The entry starts another install, profile or client.".to_owned()
        }
        ClientState::Blocked => row
            .error
            .as_ref()
            .map_or_else(String::new, |error| error.message.clone()),
        _ => row.note.unwrap_or_default().to_owned(),
    }
}

/// Whether the line under a client's name reports a problem. An old refusal stays on record, so
/// it counts only on a row that shows it.
fn is_warning(row: &ClientStatus) -> bool {
    row.state == ClientState::Blocked
        || (matches!(row.state, ClientState::Configured | ClientState::Manual)
            && row.activity.is_refused())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Change {
    Connect,
    Repair,
    Disconnect,
}

/// A change a person is reading before agreeing to it.
struct Proposal {
    change: Change,
    client: ClientId,
    name: &'static str,
    note: Option<&'static str>,
    path: Option<PathBuf>,
    entry: String,
    uses_command: bool,
    /// Keyboard focus has moved into the sheet. It lands on Cancel, so that a stray Return or
    /// Space never makes a change a person has not read.
    focused: bool,
}

impl Proposal {
    fn new(change: Change, row: &ClientStatus) -> Self {
        Self {
            change,
            client: row.client,
            name: row.name,
            note: row.note,
            path: row.path.clone(),
            entry: row.snippet.clone().unwrap_or_default(),
            uses_command: row.uses_command,
            focused: false,
        }
    }

    fn explanation(&self) -> String {
        let file = self
            .path
            .as_ref()
            .map_or_else(|| "its file".to_owned(), |path| path.display().to_string());
        const KEPT: &str = "Nothing else in the file changes. An existing file keeps its original beside it, with the ending .huda-backup.";
        match (self.change, self.uses_command) {
            (Change::Connect, true) => format!(
                "Huda runs this command of {}, which changes {file}.",
                self.name
            ),
            (Change::Repair, true) => format!(
                "Huda removes its old entry with {}'s own command, then runs this one, which changes {file}.",
                self.name
            ),
            (Change::Connect, false) => format!("Huda adds this entry to {file}. {KEPT}"),
            (Change::Repair, false) => {
                format!("Huda replaces its entry in {file} with this one. {KEPT}")
            }
            (Change::Disconnect, _) => format!(
                "Huda removes only its own entry from {file}. A backup is not restored, so your later changes stay."
            ),
        }
    }
}

struct Outcome {
    server: Option<Server>,
    rows: Vec<ClientStatus>,
    message: String,
    failed: bool,
}

fn refreshed(profile: &Profile, message: String, failed: bool) -> Outcome {
    outcome(profile, Server::of_this_install(profile), message, failed)
}

/// What a refresh shows. An install that cannot be found, such as one whose helper is missing
/// beside the desktop, is a failure with its reason, and no rows.
fn outcome(
    profile: &Profile,
    found: huda::error::Result<Server>,
    message: String,
    failed: bool,
) -> Outcome {
    match found {
        Ok(server) => Outcome {
            rows: clients::status(&server, profile),
            server: Some(server),
            message,
            failed,
        },
        Err(error) => Outcome {
            server: None,
            rows: Vec::new(),
            message: error.to_string(),
            failed: true,
        },
    }
}

fn applied(profile: &Profile, server: &Server, change: Change, client: ClientId) -> Outcome {
    let result = match change {
        Change::Connect | Change::Repair => clients::connect(client, server),
        Change::Disconnect => clients::disconnect(client),
    };
    let message = match (result, change) {
        (Ok(()), Change::Disconnect) => "Done. Restart the client so it lets go of Huda.",
        (Ok(()), _) => {
            "Done. Restart the client, then check its row: only its use of Huda proves it works."
        }
        (Err(error), _) => return refreshed(profile, error.to_string(), true),
    };
    refreshed(profile, message.to_owned(), false)
}

/// What a click in a row asks for, handled after the rows are drawn.
enum Request {
    Propose(Proposal),
    Copy(String),
}

pub struct Panel {
    profile: Profile,
    context: egui::Context,
    opened: bool,
    worker: Option<mpsc::Receiver<Outcome>>,
    server: Option<Server>,
    rows: Vec<ClientStatus>,
    proposal: Option<Proposal>,
    message: String,
    failed: bool,
    search: String,
}

/// Goes after a row is drawn, so a row can ask without holding the panel.
fn more_menu(ui: &mut egui::Ui, row: &ClientStatus, request: &mut Option<Request>) {
    if row.snippet.is_none() && row.link.is_none() {
        return;
    }
    let menu = ui.menu_button("…", |ui| {
        if let Some(snippet) = &row.snippet {
            if ui.button("Copy Entry").clicked() {
                *request = Some(Request::Copy(snippet.clone()));
                ui.close();
            }
        }
        if let Some(link) = row.link {
            if ui.button("Copy Link").clicked() {
                *request = Some(Request::Copy(link.to_owned()));
                ui.close();
            }
        }
        if row.state == ClientState::NeedsRepair && ui.button("Disconnect…").clicked() {
            *request = Some(Request::Propose(Proposal::new(Change::Disconnect, row)));
            ui.close();
        }
    });
    widgets::labelled(&menu.response, &format!("More for {}", row.name));
    menu.response.on_hover_text("More");
}

impl Panel {
    pub fn new(profile: Profile, context: egui::Context) -> Self {
        Self {
            profile,
            context,
            opened: false,
            worker: None,
            server: None,
            rows: Vec::new(),
            proposal: None,
            message: String::new(),
            failed: false,
            search: String::new(),
        }
    }

    fn start(&mut self, job: impl FnOnce(&Profile) -> Outcome + Send + 'static) {
        if self.worker.is_some() {
            return;
        }
        let (sender, receiver) = mpsc::sync_channel(1);
        let profile = self.profile.clone();
        let context = self.context.clone();
        std::thread::spawn(move || {
            let _ = sender.send(job(&profile));
            context.request_repaint();
        });
        self.worker = Some(receiver);
    }

    fn refresh(&mut self) {
        self.start(|profile| refreshed(profile, String::new(), false));
    }

    /// Rows for a test, which must not read this computer's real client files.
    #[cfg(test)]
    pub fn show_rows(&mut self, rows: Vec<ClientStatus>) {
        self.opened = true;
        self.rows = rows;
    }

    /// Opens the sheet of a change, for a test.
    #[cfg(test)]
    pub fn show_sheet(&mut self, row: &ClientStatus) {
        self.propose(Proposal::new(Change::Disconnect, row));
    }

    /// The first view of the tab reads the clients' files. A launch does not.
    pub fn open(&mut self) {
        if !self.opened {
            self.opened = true;
            self.refresh();
        }
    }

    pub fn poll(&mut self) {
        if let Some(receiver) = &self.worker {
            match receiver.try_recv() {
                Ok(outcome) => {
                    self.server = outcome.server;
                    self.rows = outcome.rows;
                    self.message = outcome.message;
                    self.failed = outcome.failed;
                    self.worker = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.message = "The client check stopped; nothing else was attempted.".into();
                    self.failed = true;
                    self.worker = None;
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
    }

    /// A new change starts clean: a message about the last one would read as being about this.
    fn propose(&mut self, proposal: Proposal) {
        self.message.clear();
        self.failed = false;
        self.proposal = Some(proposal);
    }

    fn matches(&self, row: &ClientStatus) -> bool {
        row.name
            .to_lowercase()
            .contains(&self.search.trim().to_lowercase())
    }

    fn row_ui(ui: &mut egui::Ui, row: &ClientStatus, now_ms: u64) -> Option<Request> {
        let mut request = None;
        let detail = detail(row, now_ms);
        widgets::row_with_trailing(
            ui,
            |ui| {
                widgets::row_title(ui, row.name);
                if is_warning(row) {
                    widgets::row_caution(ui, tone(row), &detail);
                } else {
                    widgets::row_detail(ui, &detail);
                }
                // The file matters when it is the problem. Otherwise the sheet names it.
                if row.state == ClientState::Blocked {
                    if let Some(path) = &row.path {
                        widgets::row_detail(ui, &path.display().to_string());
                    }
                }
            },
            |ui| {
                widgets::status(ui, tone(row), state_label(row.state));
                let action = match row.state {
                    ClientState::Available => Some((Change::Connect, "Connect…", true)),
                    ClientState::NeedsRepair => Some((Change::Repair, "Repair…", true)),
                    ClientState::Configured => Some((Change::Disconnect, "Disconnect…", false)),
                    _ => None,
                };
                if let Some((change, label, primary)) = action {
                    let button = if primary {
                        widgets::primary_button(ui, label)
                    } else {
                        ui.button(label)
                    };
                    // Read aloud, ten buttons called Connect say nothing.
                    let name = label.trim_end_matches('…');
                    widgets::labelled(&button, &format!("{name} {}", row.name));
                    if button.clicked() {
                        request = Some(Request::Propose(Proposal::new(change, row)));
                    }
                }
                more_menu(ui, row, &mut request);
            },
        );
        request
    }

    fn proposal_ui(&mut self, context: &egui::Context) {
        let Some(proposal) = &mut self.proposal else {
            return;
        };
        let mut agreed = false;
        let mut cancelled = false;
        let (action, title) = match proposal.change {
            Change::Connect => ("Connect", format!("Connect {}", proposal.name)),
            Change::Repair => ("Repair", format!("Repair {}", proposal.name)),
            Change::Disconnect => ("Disconnect", format!("Disconnect {}", proposal.name)),
        };
        let closing = widgets::sheet(context, "client-change", &title, |ui| {
            ui.label(proposal.explanation());
            if let Some(note) = proposal.note {
                widgets::row_detail(ui, note);
            }
            if proposal.change != Change::Disconnect {
                ui.add_space(4.0);
                let mut entry = proposal.entry.as_str();
                let field = ui.add(
                    egui::TextEdit::multiline(&mut entry)
                        .code_editor()
                        .desired_rows(4)
                        .desired_width(f32::INFINITY),
                );
                widgets::labelled(&field, "The entry this change writes");
            }
            ui.add_space(10.0);
            let destructive = proposal.change == Change::Disconnect;
            let focused = &mut proposal.focused;
            widgets::row_with_trailing(
                ui,
                |_| {},
                |ui| {
                    let cancel = ui.button("Cancel");
                    if !*focused {
                        *focused = true;
                        cancel.request_focus();
                    }
                    cancelled = cancel.clicked();
                    agreed = if destructive {
                        widgets::destructive_button(ui, action).clicked()
                    } else {
                        widgets::primary_button(ui, action).clicked()
                    };
                },
            );
        });
        if agreed {
            if let (Some(proposal), Some(server)) = (self.proposal.take(), self.server.clone()) {
                self.start(move |profile| {
                    applied(profile, &server, proposal.change, proposal.client)
                });
            }
        } else if cancelled || closing {
            self.proposal = None;
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        widgets::row_caution(ui, Tone::Attention, clients::PROTOCOL_NOTE);
        widgets::row_detail(
            ui,
            "An entry holds a command and no secret: the client starts Huda itself, and Huda reads its credential from the OS secret store.",
        );
        if !self.profile.database().exists() {
            widgets::row_detail(
                ui,
                "Initialize the profile in Relation Space first, or a client's tools will report that it is missing.",
            );
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let search = ui.add(
                egui::TextEdit::singleline(&mut self.search)
                    .hint_text("Search clients")
                    .desired_width(240.0)
                    .margin(egui::vec2(8.0, 6.0)),
            );
            widgets::labelled(&search, "Search clients");
            if ui
                .add_enabled(self.worker.is_none(), egui::Button::new("Refresh"))
                .clicked()
            {
                self.refresh();
            }
            if self.worker.is_some() {
                ui.spinner();
            }
        });
        if !self.message.is_empty() {
            let tone = if self.failed {
                Tone::Problem
            } else {
                Tone::Quiet
            };
            widgets::notice(ui, tone, &self.message);
        }
        ui.add_space(4.0);
        self.proposal_ui(&ui.ctx().clone());
        let now = now_ms();
        let mut request = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .id_salt("clients")
            .show(ui, |ui| {
                widgets::leave_room_for_scrollbar(ui);
                for (index, title) in GROUPS.iter().enumerate() {
                    let rows: Vec<_> = self
                        .rows
                        .iter()
                        .filter(|row| group(row.state) == index && self.matches(row))
                        .collect();
                    if rows.is_empty() {
                        continue;
                    }
                    ui.add_enabled_ui(self.worker.is_none(), |ui| {
                        widgets::section(ui, Some(title), |section| {
                            for row in rows {
                                if let Some(asked) = section.row(|ui| Self::row_ui(ui, row, now)) {
                                    request = Some(asked);
                                }
                            }
                        });
                    });
                }
            });
        match request {
            Some(Request::Propose(proposal)) => self.propose(proposal),
            Some(Request::Copy(text)) => {
                self.message = "Copied.".into();
                self.failed = false;
                ui.ctx().copy_text(text);
            }
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(state: ClientState, activity: Activity) -> ClientStatus {
        ClientStatus {
            client: ClientId::Cursor,
            name: "Cursor",
            state,
            note: Some("A note."),
            error: None,
            path: Some(PathBuf::from("/home/me/.cursor/mcp.json")),
            snippet: Some("{ }".to_owned()),
            link: Some("https://cursor.com/"),
            uses_command: false,
            activity,
        }
    }

    const MINUTE: u64 = 60_000;
    const NOW: u64 = 100 * 24 * 60 * MINUTE;

    #[test]
    fn a_time_reads_in_its_largest_unit() {
        assert_eq!(ago(NOW, NOW), "just now");
        assert_eq!(ago(NOW - MINUTE, NOW), "1 minute ago");
        assert_eq!(ago(NOW - 5 * MINUTE, NOW), "5 minutes ago");
        assert_eq!(ago(NOW - 60 * MINUTE, NOW), "1 hour ago");
        assert_eq!(ago(NOW - 3 * 1440 * MINUTE, NOW), "3 days ago");
        assert_eq!(ago(NOW + MINUTE, NOW), "just now");
    }

    #[test]
    fn configured_alone_never_claims_the_client_works() {
        let text = detail(&row(ClientState::Configured, Activity::default()), NOW);
        assert!(text.contains("has not used Huda yet"), "{text}");
    }

    #[test]
    fn a_refused_handshake_is_named_and_a_later_call_replaces_it() {
        let refused = Activity {
            used_ms: None,
            refused_ms: Some(NOW - MINUTE),
        };
        let text = detail(&row(ClientState::Configured, refused), NOW);
        assert!(text.contains("2026-07-28"), "{text}");
        let works = Activity {
            used_ms: Some(NOW),
            refused_ms: Some(NOW - MINUTE),
        };
        assert_eq!(
            detail(&row(ClientState::Configured, works), NOW),
            "Last used just now."
        );
    }

    #[test]
    fn every_state_belongs_to_a_group_and_has_words() {
        for state in [
            ClientState::NotInstalled,
            ClientState::Available,
            ClientState::Configured,
            ClientState::NeedsRepair,
            ClientState::Blocked,
            ClientState::Manual,
            ClientState::Hosted,
        ] {
            assert!(group(state) < GROUPS.len());
            assert!(!state_label(state).is_empty());
        }
    }

    #[test]
    fn a_proposal_names_the_file_and_the_limits_of_the_change() {
        let connect = Proposal::new(
            Change::Connect,
            &row(ClientState::Available, Activity::default()),
        );
        let text = connect.explanation();
        assert!(text.contains("/home/me/.cursor/mcp.json"), "{text}");
        assert!(text.contains(".huda-backup"), "{text}");
        let disconnect = Proposal::new(
            Change::Disconnect,
            &row(ClientState::Configured, Activity::default()),
        );
        assert!(disconnect.explanation().contains("only its own entry"));
    }

    #[test]
    fn a_repair_says_it_replaces_and_a_new_file_is_not_promised_a_backup() {
        let repair = Proposal::new(
            Change::Repair,
            &row(ClientState::NeedsRepair, Activity::default()),
        );
        assert!(repair.explanation().contains("replaces its entry"));
        let connect = Proposal::new(
            Change::Connect,
            &row(ClientState::Available, Activity::default()),
        );
        assert!(connect.explanation().contains("An existing file keeps"));
        let mut by_command = row(ClientState::NeedsRepair, Activity::default());
        by_command.uses_command = true;
        let text = Proposal::new(Change::Repair, &by_command).explanation();
        assert!(text.contains("removes its old entry"), "{text}");
    }

    #[test]
    fn an_old_refusal_colours_only_a_row_that_still_shows_it() {
        let refused = Activity {
            used_ms: None,
            refused_ms: Some(NOW),
        };
        assert!(is_warning(&row(ClientState::Configured, refused)));
        assert!(is_warning(&row(ClientState::Manual, refused)));
        assert!(is_warning(&row(ClientState::Blocked, Activity::default())));
        for state in [ClientState::Available, ClientState::NeedsRepair] {
            assert!(!is_warning(&row(state, refused)), "{state:?}");
        }
        assert!(!is_warning(&row(
            ClientState::Configured,
            Activity::default()
        )));
    }

    #[test]
    fn a_new_proposal_clears_the_message_of_the_last_change() {
        let folder = tempfile::tempdir().unwrap();
        let profile = Profile::new(folder.path().join("profile")).unwrap();
        let mut panel = Panel::new(profile, egui::Context::default());
        panel.message = "CLIENT_RUNNING: Quit this client first".into();
        panel.failed = true;
        panel.propose(Proposal::new(
            Change::Connect,
            &row(ClientState::Available, Activity::default()),
        ));
        assert!(panel.message.is_empty());
        assert!(!panel.failed);
        assert!(panel.proposal.is_some());
    }

    #[test]
    fn a_failed_check_is_marked_as_failed() {
        // Whether a test binary has a helper beside it depends on how Cargo lays out its output
        // on each system, so the broken install is given rather than looked for.
        let folder = tempfile::tempdir().unwrap();
        let profile = Profile::new(folder.path().join("profile")).unwrap();
        let missing = huda::error::Error::new(
            "CORE_UNAVAILABLE",
            "The packaged huda helper is missing beside the desktop executable",
        );
        let outcome = outcome(&profile, Err(missing), String::new(), false);
        assert!(outcome.failed);
        assert!(outcome.rows.is_empty() && outcome.server.is_none());
        assert!(
            outcome.message.contains("helper is missing"),
            "{}",
            outcome.message
        );
    }

    #[test]
    fn the_tab_draws_every_kind_of_row_without_a_window() {
        let folder = tempfile::tempdir().unwrap();
        let profile = Profile::new(folder.path().join("profile")).unwrap();
        let context = egui::Context::default();
        crate::theme::apply(&context);
        let mut panel = Panel::new(profile, context.clone());
        panel.rows = [
            ClientState::NotInstalled,
            ClientState::Available,
            ClientState::Configured,
            ClientState::NeedsRepair,
            ClientState::Blocked,
            ClientState::Manual,
            ClientState::Hosted,
        ]
        .map(|state| row(state, Activity::default()))
        .to_vec();
        panel.proposal = Some(Proposal::new(
            Change::Connect,
            &row(ClientState::Available, Activity::default()),
        ));
        let mut output = context.run_ui(egui::RawInput::default(), |ui| panel.ui(ui));
        assert!(!output.shapes.is_empty());
        // No renderer takes the font textures here.
        output.textures_delta.clear();
    }

    #[test]
    fn opening_the_tab_twice_reads_the_files_once() {
        let folder = tempfile::tempdir().unwrap();
        let profile = Profile::new(folder.path().join("profile")).unwrap();
        let mut panel = Panel::new(profile, egui::Context::default());
        assert!(panel.worker.is_none());
        panel.open();
        assert!(panel.worker.is_some());
        panel.worker = None;
        panel.open();
        assert!(panel.worker.is_none());
    }
}
