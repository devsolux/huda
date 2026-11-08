//! Connecting AI clients: puts `huda mcp stdio` into a client's own configuration and takes it
//! out again. This works on files of other products and never on the profile's data, so it is
//! not a handler: no MCP tool or REST route reaches it, and a model cannot change what a client
//! starts. Every change is shown first, backed up, and made only on an explicit request.
//!
//! A client's entry is a command and arguments and never a secret: the started helper reads the
//! `mcp` credential from the OS secret store as any other `huda mcp stdio` does.
pub mod activity;
mod catalog;
mod json_config;
mod safe_write;
mod status;
mod toml_config;
mod vendor_cli;

use crate::error::{require, Error, Result};
use crate::runtime::{helper, Profile};
use catalog::{entry as catalog_entry, Entry, Folders, Format, Method, MCP_SERVERS};
use serde::Serialize;
use status::{entry_state, EntryState};
use std::path::{Path, PathBuf};

pub use activity::{Activity, Observer};
pub use catalog::ClientId;
pub use status::{status, ClientState, ClientStatus};

/// The name clients show for this server.
const SERVER_NAME: &str = "Huda";
const CLIENT_FLAG: &str = "--client";
const BOM: char = '\u{feff}';

/// What a person has to know about every row, until the open decision about older clients is
/// settled: a configured client can still fail, and the file cannot show it.
pub const PROTOCOL_NOTE: &str = "Huda speaks only MCP 2026-07-28. A client that opens with `initialize` is refused and shows Huda as failed; its row says so after its first attempt.";

fn invalid_config() -> Error {
    Error::new(
        "CLIENT_CONFIG_INVALID",
        "This client's file is not in the expected form, so it was left as it was",
    )
}

fn not_automatic() -> Error {
    Error::new(
        "CLIENT_NOT_AUTOMATIC",
        "Huda does not change this client by itself here; paste its entry by hand",
    )
}

fn not_installed() -> Error {
    Error::new(
        "CLIENT_NOT_INSTALLED",
        "This client is not on this computer, so nothing was written for it",
    )
}

/// A byte order mark is not part of either format, so editors work on the text after it.
fn split_bom(text: &str) -> (&str, &str) {
    text.strip_prefix(BOM)
        .map_or(("", text), |rest| (&text[..BOM.len_utf8()], rest))
}

/// What a client's configuration starts: this install's helper, for one profile.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Server {
    program: PathBuf,
    /// Set for a profile other than the default, so the entry keeps serving that profile.
    profile: Option<PathBuf>,
}

impl Server {
    /// This install's helper for `profile`.
    pub fn of_this_install(profile: &Profile) -> Result<Self> {
        let is_default = Profile::default_path()
            .ok()
            .and_then(|path| std::fs::canonicalize(path).ok())
            .is_some_and(|path| path == profile.directory());
        Ok(Self {
            program: without_verbatim_prefix(helper::real_executable()?),
            profile: (!is_default).then(|| profile.directory().to_path_buf()),
        })
    }

    fn program(&self) -> &Path {
        &self.program
    }

    fn command(&self) -> String {
        self.program.display().to_string()
    }

    /// The arguments of `client`'s entry. `--client` tells Huda which row is talking.
    fn args(&self, client: ClientId) -> Vec<String> {
        let mut args = Vec::new();
        if let Some(profile) = &self.profile {
            args.push("--profile".to_owned());
            args.push(profile.display().to_string());
        }
        args.extend(["mcp", "stdio", CLIENT_FLAG, client.id()].map(str::to_owned));
        args
    }

    /// A client keeps this path after Huda exits, so a path that goes away is refused, as it is
    /// for a login entry.
    fn require_lasting(&self) -> Result<()> {
        let text = self
            .program
            .to_str()
            .ok_or_else(|| Error::invalid("A client's entry needs a Unicode path to Huda"))?;
        require(
            self.program.is_absolute() && !text.chars().any(char::is_control),
            "INVALID_INPUT",
            "A client's entry needs an absolute path to Huda without control characters",
        )?;
        require(
            !text.starts_with("/Volumes/") && !text.contains("/AppTranslocation/"),
            "INVALID_INPUT",
            "Move Huda.app to Applications before connecting a client",
        )
    }
}

/// `canonicalize` gives a verbatim prefix on Windows, which a client could not start.
fn without_verbatim_prefix(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    match text.strip_prefix(r"\\?\") {
        Some(rest) => PathBuf::from(rest),
        None => path,
    }
}

/// This server's entry in a client's configuration.
struct ServerEntry<'a> {
    name: &'a str,
    command: &'a str,
    args: Vec<String>,
}

impl ServerEntry<'static> {
    /// An entry that only tells whether a file could be edited.
    fn probe() -> Self {
        Self {
            name: SERVER_NAME,
            command: "",
            args: Vec::new(),
        }
    }
}

fn with_server(format: Format, current: &str, entry: &ServerEntry) -> Result<String> {
    match format {
        Format::Json(shape) => json_config::with_server(current, shape, entry),
        Format::Toml => toml_config::with_server(current, entry),
    }
}

fn without_server(format: Format, current: &str) -> Result<Option<String>> {
    match format {
        Format::Json(shape) => json_config::without_server(current, shape, SERVER_NAME),
        Format::Toml => toml_config::without_server(current, SERVER_NAME),
    }
}

/// What the user copies, in the form the client reads, even where Huda does not write it itself.
/// A client without a known form gets the one most clients read.
fn snippet_for(entry: &Entry, server: &Server) -> String {
    let command = server.command();
    let server_entry = ServerEntry {
        name: SERVER_NAME,
        command: &command,
        args: server.args(entry.client),
    };
    match entry.declared_method() {
        Method::File(_, Format::Json(shape)) => json_config::snippet(shape, &server_entry),
        Method::File(_, Format::Toml) => toml_config::snippet(&server_entry),
        Method::Cli { program, .. } => format!(
            "{program} mcp add --scope user --transport stdio {SERVER_NAME} -- \"{command}\" {}",
            server_entry.args.join(" ")
        ),
        Method::Manual | Method::Hosted => json_config::snippet(MCP_SERVERS, &server_entry),
    }
}

fn connect_file(format: Format, path: &Path, server: &Server, client: ClientId) -> Result<()> {
    let command = server.command();
    let entry = ServerEntry {
        name: SERVER_NAME,
        command: &command,
        args: server.args(client),
    };
    safe_write::update(path, |current| {
        with_server(format, current, &entry).map(Some)
    })?;
    require(
        holds_entry(format, path, server, client)?,
        "CLIENT_CONFIG_UNSAFE",
        "The change could not be read back from this client's file",
    )
}

fn holds_entry(format: Format, path: &Path, server: &Server, client: ClientId) -> Result<bool> {
    let written = safe_write::read_or_empty(path)?;
    Ok(entry_state(format, &written, server, client) == EntryState::Current)
}

/// Connects through a client's own command, which keeps its own file consistent. An entry
/// already there goes first, because adding refuses a name in use.
fn connect_cli(program: &Path, status: &Path, server: &Server, client: ClientId) -> Result<()> {
    let format = Format::Json(MCP_SERVERS);
    let current = safe_write::read_or_empty(status)?;
    if status::server_command(format, &current).is_some() {
        vendor_cli::run(program, &["mcp", "remove", SERVER_NAME, "-s", "user"])?;
    }
    let command = server.command();
    let args = server.args(client);
    let mut arguments = vec![
        "mcp",
        "add",
        "--scope",
        "user",
        "--transport",
        "stdio",
        SERVER_NAME,
        "--",
        &command,
    ];
    arguments.extend(args.iter().map(String::as_str));
    vendor_cli::run(program, &arguments)?;
    if holds_entry(format, status, server, client)? {
        Ok(())
    } else {
        Err(vendor_cli::unavailable())
    }
}

/// A client that writes its file back from memory would undo a change, and a change that seems
/// to work and then vanishes is worse than asking the user to close it. Huda never closes
/// another product's process.
fn require_closed(entry: &Entry, is_running: impl Fn(&str) -> bool) -> Result<()> {
    require(
        !entry.rewrites_file_as.is_some_and(is_running),
        "CLIENT_RUNNING",
        "Quit this client first: it would undo the change when it next saves",
    )
}

fn apply_connect(entry: &Entry, folders: &Folders, server: &Server) -> Result<()> {
    let method = entry.method();
    if matches!(method, Method::Manual | Method::Hosted) {
        return Err(not_automatic());
    }
    server.require_lasting()?;
    if !folders.has(entry) {
        return Err(not_installed());
    }
    require_closed(entry, vendor_cli::is_running)?;
    match method {
        Method::File(place, format) => {
            let path = folders.path(place).ok_or_else(not_automatic)?;
            connect_file(format, &path, server, entry.client)
        }
        Method::Cli { program, status } => {
            let path = folders.path(status).ok_or_else(not_automatic)?;
            let program =
                vendor_cli::find(program, folders.home()).ok_or_else(vendor_cli::unavailable)?;
            connect_cli(&program, &path, server, entry.client)
        }
        Method::Manual | Method::Hosted => Err(not_automatic()),
    }
}

/// Takes this server out of the client's configuration. Every other entry stays. The backup is
/// not restored, because that would undo the user's later changes.
fn apply_disconnect(entry: &Entry, folders: &Folders) -> Result<()> {
    require_closed(entry, vendor_cli::is_running)?;
    match entry.method() {
        Method::File(place, format) => {
            let path = folders.path(place).ok_or_else(not_automatic)?;
            safe_write::update(&path, |current| without_server(format, current))
        }
        Method::Cli { program, .. } => {
            let program =
                vendor_cli::find(program, folders.home()).ok_or_else(vendor_cli::unavailable)?;
            vendor_cli::run(&program, &["mcp", "remove", SERVER_NAME, "-s", "user"])
        }
        Method::Manual | Method::Hosted => Err(not_automatic()),
    }
}

fn entry_of(client: ClientId) -> Result<&'static Entry> {
    catalog_entry(client).ok_or_else(not_automatic)
}

fn folders() -> Result<Folders> {
    Folders::of_this_user().ok_or_else(|| Error::invalid("The home folder is unavailable"))
}

/// What connecting or disconnecting would do, so a person can read it before agreeing.
#[derive(Clone, Debug, Serialize)]
pub struct Preview {
    pub client: ClientId,
    pub name: &'static str,
    /// The file that changes. For a client's own command, the file that command changes.
    pub path: Option<PathBuf>,
    /// The entry this concerns, or the command that adds it.
    pub entry: Option<String>,
    pub uses_command: bool,
    /// Whether Huda can make this change itself here. Otherwise `change` is pasted by hand.
    pub automatic: bool,
}

/// Shows a change without making it. Writes nothing.
pub fn preview(client: ClientId, server: &Server) -> Result<Preview> {
    let entry = entry_of(client)?;
    let row = status::client_status(entry, entry.method(), &folders()?, server);
    Ok(Preview {
        client,
        name: entry.name,
        path: row.path,
        entry: row.snippet,
        uses_command: row.uses_command,
        automatic: matches!(entry.method(), Method::File(..) | Method::Cli { .. }),
    })
}

/// Writes the client's configuration after the user saw the change, keeping a backup.
pub fn connect(client: ClientId, server: &Server) -> Result<()> {
    apply_connect(entry_of(client)?, &folders()?, server)
}

/// Takes this server out of the client's configuration.
pub fn disconnect(client: ClientId) -> Result<()> {
    apply_disconnect(entry_of(client)?, &folders()?)
}

#[cfg(test)]
mod tests;
