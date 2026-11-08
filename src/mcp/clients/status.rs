//! What is listed for each client. Finding this out reads files and nothing else: no process
//! starts, and nothing runs until someone asks for the list.
use super::activity::{self, Activity};
use super::catalog::{ClientId, Entry, Folders, Format, Method, Place, CATALOG, MCP_SERVERS};
use super::{json_config, safe_write, snippet_for, toml_config, Server, ServerEntry, SERVER_NAME};
use crate::error::Error;
use crate::runtime::Profile;
use serde::Serialize;
use std::path::{Path, PathBuf};

/// What a client's file holds for this server.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryState {
    /// No entry, or a file too broken to hold one.
    Missing,
    /// The entry starts this install's helper for this client and this profile.
    Current,
    /// The entry starts something else, or names another client.
    Stale,
}

pub(super) fn server_command(format: Format, current: &str) -> Option<String> {
    match format {
        Format::Json(shape) => json_config::server_command(current, shape, SERVER_NAME),
        Format::Toml => toml_config::server_command(current, SERVER_NAME),
    }
}

fn server_args(format: Format, current: &str) -> Vec<String> {
    match format {
        Format::Json(shape) => json_config::server_args(current, shape, SERVER_NAME),
        Format::Toml => toml_config::server_args(current, SERVER_NAME),
    }
}

/// Whether a client's file holds the entry this install would write.
pub fn entry_state(format: Format, current: &str, server: &Server, client: ClientId) -> EntryState {
    let Some(command) = server_command(format, current) else {
        return EntryState::Missing;
    };
    if Path::new(&command) == server.program()
        && server_args(format, current) == server.args(client)
    {
        EntryState::Current
    } else {
        EntryState::Stale
    }
}

/// The state of one row. `Configured` says what a file holds, not that the client works: that
/// is what the row's activity is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientState {
    /// No sign of the client on this computer.
    NotInstalled,
    /// Installed, without an entry.
    Available,
    /// The file holds exactly the entry this install would write.
    Configured,
    /// The entry is out of date. Connecting repairs it.
    NeedsRepair,
    /// The file cannot be read or edited, so it is left alone. `error` says why.
    Blocked,
    /// The user pastes the snippet.
    Manual,
    /// The client cannot reach a local server.
    Hosted,
}

/// One row of the list.
#[derive(Clone, Debug, Serialize)]
pub struct ClientStatus {
    pub client: ClientId,
    pub name: &'static str,
    pub state: ClientState,
    pub note: Option<&'static str>,
    pub error: Option<Error>,
    /// The file this client's connection lives in.
    pub path: Option<PathBuf>,
    /// What connecting adds, and what the user pastes by hand.
    pub snippet: Option<String>,
    /// Where to get the client, or how to set it up.
    pub link: Option<&'static str>,
    /// The client's own command makes the change, so the file is only read.
    pub uses_command: bool,
    /// When this client last called a tool, and when Huda last refused its handshake.
    pub activity: Activity,
}

/// A file that cannot be edited keeps its row, with the reason.
fn file_state(
    entry: &Entry,
    folders: &Folders,
    format: Format,
    path: &Path,
    server: &Server,
) -> (ClientState, Option<Error>) {
    let current = match safe_write::read_or_empty(path) {
        Ok(current) => current,
        Err(error) => return (ClientState::Blocked, Some(error)),
    };
    let probe = ServerEntry::probe();
    if let Err(error) = super::with_server(format, &current, &probe) {
        return (ClientState::Blocked, Some(error));
    }
    let state = match entry_state(format, &current, server, entry.client) {
        EntryState::Current => ClientState::Configured,
        EntryState::Stale => ClientState::NeedsRepair,
        EntryState::Missing if folders.has(entry) => ClientState::Available,
        EntryState::Missing => ClientState::NotInstalled,
    };
    (state, None)
}

fn row(entry: &Entry, server: &Server, state: ClientState) -> ClientStatus {
    ClientStatus {
        client: entry.client,
        name: entry.name,
        state,
        note: entry.note,
        error: None,
        path: None,
        snippet: Some(snippet_for(entry, server)),
        link: entry.link,
        uses_command: false,
        activity: Activity::default(),
    }
}

fn status_file(
    entry: &Entry,
    folders: &Folders,
    place: Place,
    format: Format,
    server: &Server,
) -> ClientStatus {
    let path = folders.path(place);
    let (state, error) = path
        .as_deref()
        .map_or((ClientState::NotInstalled, None), |path| {
            file_state(entry, folders, format, path, server)
        });
    ClientStatus {
        error,
        path,
        ..row(entry, server, state)
    }
}

/// The row of one client connected by `method`.
pub fn client_status(
    entry: &Entry,
    method: Method,
    folders: &Folders,
    server: &Server,
) -> ClientStatus {
    match method {
        Method::File(place, format) => status_file(entry, folders, place, format, server),
        Method::Cli { status, .. } => ClientStatus {
            uses_command: true,
            ..status_file(entry, folders, status, Format::Json(MCP_SERVERS), server)
        },
        Method::Manual => row(entry, server, ClientState::Manual),
        Method::Hosted => ClientStatus {
            snippet: None,
            ..row(entry, server, ClientState::Hosted)
        },
    }
}

/// Every row. Without a home folder nothing can be connected, so every client is pasted by hand.
pub fn status(server: &Server, profile: &Profile) -> Vec<ClientStatus> {
    let folders = Folders::of_this_user();
    CATALOG
        .iter()
        .map(|entry| {
            let mut row = match &folders {
                Some(folders) => client_status(entry, entry.method(), folders, server),
                None if entry.method() == Method::Hosted => ClientStatus {
                    snippet: None,
                    ..row(entry, server, ClientState::Hosted)
                },
                None => row(entry, server, ClientState::Manual),
            };
            row.activity = activity::read(profile, entry.client);
            row
        })
        .collect()
}
