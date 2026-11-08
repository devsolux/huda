//! Every AI client Huda lists. The list is compiled in and never fetched, because a list from
//! elsewhere would decide which command runs on this user's computer.
use serde::Serialize;
use std::path::{Path, PathBuf};

/// A client in the catalog.
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientId {
    ClaudeDesktop,
    Codex,
    ClaudeCode,
    Cursor,
    VsCode,
    Antigravity,
    Kiro,
    CopilotCli,
    Goose,
    LmStudio,
    Continue,
    KiloCode,
    Amp,
    JetbrainsAi,
    Junie,
    Warp,
    Other,
    ChatgptChat,
    ClaudeWeb,
    M365Copilot,
}

impl ClientId {
    /// The name a command line and a client's configuration use.
    pub const fn id(self) -> &'static str {
        match self {
            Self::ClaudeDesktop => "claude_desktop",
            Self::Codex => "codex",
            Self::ClaudeCode => "claude_code",
            Self::Cursor => "cursor",
            Self::VsCode => "vs_code",
            Self::Antigravity => "antigravity",
            Self::Kiro => "kiro",
            Self::CopilotCli => "copilot_cli",
            Self::Goose => "goose",
            Self::LmStudio => "lm_studio",
            Self::Continue => "continue",
            Self::KiloCode => "kilo_code",
            Self::Amp => "amp",
            Self::JetbrainsAi => "jetbrains_ai",
            Self::Junie => "junie",
            Self::Warp => "warp",
            Self::Other => "other",
            Self::ChatgptChat => "chatgpt_chat",
            Self::ClaudeWeb => "claude_web",
            Self::M365Copilot => "m365_copilot",
        }
    }

    /// The client with this name. A configuration can hold any name, so this may find none.
    pub fn from_id(id: &str) -> Option<Self> {
        CATALOG
            .iter()
            .map(|entry| entry.client)
            .find(|client| client.id() == id)
    }
}

/// How a client's JSON file holds a server.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JsonShape {
    /// The object that holds the servers.
    pub servers: &'static str,
    /// The entry's `type`, for a client that asks for one.
    pub kind: Option<&'static str>,
    /// The client hides a server's tools unless the entry lists them.
    pub all_tools: bool,
}

/// The form most clients share, and what an unknown client is offered to paste.
pub const MCP_SERVERS: JsonShape = JsonShape {
    servers: "mcpServers",
    kind: None,
    all_tools: false,
};
const VS_CODE: JsonShape = JsonShape {
    servers: "servers",
    kind: Some("stdio"),
    all_tools: false,
};
const COPILOT_CLI: JsonShape = JsonShape {
    servers: "mcpServers",
    kind: Some("local"),
    all_tools: true,
};

/// The format of a client's file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Json(JsonShape),
    Toml,
}

/// A place below a folder every system has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    /// Below the home folder.
    Home(&'static str),
    /// Below `~/Library/Application Support`, `%APPDATA%` or `~/.config`.
    AppData(&'static str),
}

/// How a client gets connected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    /// Huda changes the client's file.
    File(Place, Format),
    /// The client's own command changes a file its running process rewrites. Huda only reads it.
    Cli {
        program: &'static str,
        status: Place,
    },
    /// The user pastes what Huda shows.
    Manual,
    /// The vendor's cloud connects to a public server, so a local one cannot be reached.
    Hosted,
}

/// One client of the catalog.
#[derive(Clone, Copy, Debug)]
pub struct Entry {
    pub client: ClientId,
    /// What a person calls it.
    pub name: &'static str,
    /// What a person should know before connecting it.
    pub note: Option<&'static str>,
    /// Any of these existing means the client is installed.
    pub markers: &'static [Place],
    /// macOS applications that mean the same.
    pub apps: &'static [&'static str],
    /// Where to get the client, or how to set it up. Never built from input.
    pub link: Option<&'static str>,
    /// The process that keeps this client's file in memory and writes it back while it runs,
    /// which would undo a change. Such a client is only changed while it is closed.
    pub rewrites_file_as: Option<&'static str>,
    method: Method,
}

/// Huda edits a file by itself on macOS only, the one system these paths were collected for.
/// Elsewhere every client is pasted by hand, so an unknown layout never gets a wrong write.
const ONE_CLICK: bool = cfg!(target_os = "macos");

impl Entry {
    /// How this client is connected on this system.
    pub const fn method(&self) -> Method {
        self.method_on(ONE_CLICK)
    }

    /// How this client is connected where Huda connects it by itself, which says what form its
    /// configuration has on every system.
    pub const fn declared_method(&self) -> Method {
        self.method
    }

    const fn method_on(&self, one_click: bool) -> Method {
        match self.method {
            Method::File(..) | Method::Cli { .. } if !one_click => Method::Manual,
            method => method,
        }
    }
}

const fn manual(client: ClientId, name: &'static str, link: &'static str) -> Entry {
    Entry {
        client,
        name,
        note: None,
        markers: &[],
        apps: &[],
        link: Some(link),
        rewrites_file_as: None,
        method: Method::Manual,
    }
}

const fn hosted(client: ClientId, name: &'static str, note: &'static str) -> Entry {
    Entry {
        client,
        name,
        note: Some(note),
        markers: &[],
        apps: &[],
        link: None,
        rewrites_file_as: None,
        method: Method::Hosted,
    }
}

/// In the order they are listed.
pub const CATALOG: [Entry; 20] = [
    Entry {
        client: ClientId::ClaudeDesktop,
        name: "Claude Desktop",
        note: Some("Close it first: it writes this file back while it runs."),
        markers: &[Place::AppData("Claude")],
        apps: &["Claude.app"],
        link: Some("https://claude.ai/download"),
        rewrites_file_as: Some("Claude"),
        method: Method::File(
            Place::AppData("Claude/claude_desktop_config.json"),
            Format::Json(MCP_SERVERS),
        ),
    },
    Entry {
        client: ClientId::Codex,
        name: "ChatGPT desktop and Codex",
        note: Some("Their local agent only, not ChatGPT web chats."),
        markers: &[Place::Home(".codex")],
        apps: &["ChatGPT.app", "Codex.app"],
        link: Some("https://learn.chatgpt.com/docs/extend/mcp?surface=cli"),
        rewrites_file_as: None,
        method: Method::File(Place::Home(".codex/config.toml"), Format::Toml),
    },
    Entry {
        client: ClientId::ClaudeCode,
        name: "Claude Code",
        note: Some("Connected through its own command, which keeps its file consistent."),
        markers: &[Place::Home(".claude.json"), Place::Home(".claude")],
        apps: &[],
        link: Some("https://claude.com/claude-code"),
        rewrites_file_as: None,
        method: Method::Cli {
            program: "claude",
            status: Place::Home(".claude.json"),
        },
    },
    Entry {
        client: ClientId::Cursor,
        name: "Cursor",
        note: None,
        markers: &[Place::Home(".cursor")],
        apps: &["Cursor.app"],
        link: Some("https://cursor.com/"),
        rewrites_file_as: None,
        method: Method::File(Place::Home(".cursor/mcp.json"), Format::Json(MCP_SERVERS)),
    },
    Entry {
        client: ClientId::VsCode,
        name: "VS Code with Copilot",
        note: Some("The default profile only. Paste the entry into any other profile."),
        markers: &[Place::AppData("Code")],
        apps: &["Visual Studio Code.app"],
        link: Some("https://code.visualstudio.com/"),
        rewrites_file_as: None,
        method: Method::File(Place::AppData("Code/User/mcp.json"), Format::Json(VS_CODE)),
    },
    Entry {
        client: ClientId::Antigravity,
        name: "Antigravity",
        note: None,
        markers: &[
            Place::Home(".gemini/config"),
            Place::Home(".gemini/antigravity"),
        ],
        apps: &["Antigravity.app"],
        link: Some("https://antigravity.google/docs/mcp"),
        rewrites_file_as: None,
        method: Method::File(
            Place::Home(".gemini/config/mcp_config.json"),
            Format::Json(MCP_SERVERS),
        ),
    },
    Entry {
        client: ClientId::Kiro,
        name: "Kiro",
        note: None,
        markers: &[Place::Home(".kiro")],
        apps: &["Kiro.app"],
        link: Some("https://kiro.dev/docs/mcp/configuration/"),
        rewrites_file_as: None,
        method: Method::File(
            Place::Home(".kiro/settings/mcp.json"),
            Format::Json(MCP_SERVERS),
        ),
    },
    Entry {
        client: ClientId::CopilotCli,
        name: "GitHub Copilot CLI",
        note: None,
        markers: &[Place::Home(".copilot")],
        apps: &[],
        link: Some(
            "https://docs.github.com/en/copilot/how-tos/copilot-cli/customize-copilot/add-mcp-servers",
        ),
        rewrites_file_as: None,
        method: Method::File(
            Place::Home(".copilot/mcp-config.json"),
            Format::Json(COPILOT_CLI),
        ),
    },
    manual(
        ClientId::Goose,
        "Goose",
        "https://block.github.io/goose/docs/getting-started/using-extensions",
    ),
    manual(
        ClientId::LmStudio,
        "LM Studio",
        "https://lmstudio.ai/docs/app/mcp",
    ),
    manual(ClientId::Continue, "Continue", "https://www.continue.dev/"),
    manual(
        ClientId::KiloCode,
        "Kilo Code",
        "https://kilo.ai/docs/automate/mcp/using-in-kilo-code",
    ),
    manual(ClientId::Amp, "Amp", "https://ampcode.com/"),
    manual(
        ClientId::JetbrainsAi,
        "JetBrains AI Assistant",
        "https://www.jetbrains.com/ai-assistant/",
    ),
    manual(
        ClientId::Junie,
        "Junie",
        "https://www.jetbrains.com/help/junie/model-context-protocol-mcp.html",
    ),
    manual(ClientId::Warp, "Warp", "https://www.warp.dev/"),
    Entry {
        client: ClientId::Other,
        name: "Other MCP client",
        note: Some("Paste this into any client that can start a local server."),
        markers: &[],
        apps: &[],
        link: None,
        rewrites_file_as: None,
        method: Method::Manual,
    },
    hosted(
        ClientId::ChatgptChat,
        "ChatGPT chat",
        "Needs a public server, and Huda stays on this computer. Use ChatGPT desktop instead.",
    ),
    hosted(
        ClientId::ClaudeWeb,
        "claude.ai, Cowork and Claude mobile",
        "Needs a public server, and Huda stays on this computer. Use Claude Desktop instead.",
    ),
    hosted(
        ClientId::M365Copilot,
        "Microsoft 365 Copilot",
        "Needs a public server, and Huda stays on this computer.",
    ),
];

/// The catalog's row of `client`.
pub fn entry(client: ClientId) -> Option<&'static Entry> {
    CATALOG.iter().find(|entry| entry.client == client)
}

/// The folders of this user that a `Place` is relative to.
#[derive(Clone, Debug)]
pub struct Folders {
    home: PathBuf,
    app_data: Option<PathBuf>,
    applications: Vec<PathBuf>,
}

impl Folders {
    /// The folders of the user running Huda.
    pub fn of_this_user() -> Option<Self> {
        let home = std::env::home_dir()?;
        let app_data = if cfg!(target_os = "macos") {
            Some(home.join("Library/Application Support"))
        } else if cfg!(windows) {
            std::env::var_os("APPDATA").map(PathBuf::from)
        } else {
            Some(home.join(".config"))
        };
        let applications = vec![PathBuf::from("/Applications"), home.join("Applications")];
        Some(Self {
            home,
            app_data,
            applications,
        })
    }

    pub fn home(&self) -> &Path {
        &self.home
    }

    /// The path of `place`, when this system has its folder.
    pub fn path(&self, place: Place) -> Option<PathBuf> {
        match place {
            Place::Home(path) => Some(self.home.join(path)),
            Place::AppData(path) => Some(self.app_data.as_ref()?.join(path)),
        }
    }

    /// Whether a client's files or application are here. Only asks the file system.
    pub fn has(&self, entry: &Entry) -> bool {
        let has_marker = entry
            .markers
            .iter()
            .filter_map(|&marker| self.path(marker))
            .any(|path| path.exists());
        has_marker
            || self
                .applications
                .iter()
                .any(|folder| entry.apps.iter().any(|app| folder.join(app).exists()))
    }
}

#[cfg(test)]
impl Folders {
    /// Folders below `home`, for a test.
    pub fn below(home: &Path) -> Self {
        Self {
            home: home.to_path_buf(),
            app_data: Some(home.join("AppData")),
            applications: vec![home.join("Applications")],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_client_has_one_row_and_a_stable_name() {
        for (index, row) in CATALOG.iter().enumerate() {
            assert!(
                CATALOG[index + 1..]
                    .iter()
                    .all(|other| other.client != row.client),
                "{:?}",
                row.client
            );
            assert_eq!(serde_json::to_value(row.client).unwrap(), row.client.id());
            assert_eq!(ClientId::from_id(row.client.id()), Some(row.client));
        }
        assert_eq!(ClientId::from_id("unknown"), None);
    }

    #[test]
    fn a_hosted_client_never_offers_a_connection() {
        for row in CATALOG
            .iter()
            .filter(|row| row.method == Method::Hosted)
            .collect::<Vec<_>>()
        {
            assert_eq!(row.method_on(true), Method::Hosted);
            assert_eq!(row.link, None);
            assert!(row.note.is_some(), "{}", row.name);
        }
        assert_eq!(
            CATALOG
                .iter()
                .filter(|row| row.method == Method::Hosted)
                .count(),
            3
        );
    }

    #[test]
    fn without_one_click_every_local_client_is_pasted_by_hand() {
        for row in &CATALOG {
            let hosted = row.method == Method::Hosted;
            assert_eq!(row.method_on(false) == Method::Hosted, hosted);
            assert!(hosted || row.method_on(false) == Method::Manual);
        }
    }

    #[test]
    fn every_link_is_https() {
        for link in CATALOG.iter().filter_map(|row| row.link) {
            assert!(link.starts_with("https://"), "{link}");
        }
    }

    #[test]
    fn a_marker_or_an_application_shows_a_client() {
        let home = tempfile::tempdir().unwrap();
        let folders = Folders::below(home.path());
        let cursor = entry(ClientId::Cursor).unwrap();
        assert!(!folders.has(cursor));
        std::fs::create_dir_all(home.path().join("Applications/Cursor.app")).unwrap();
        assert!(folders.has(cursor));
        std::fs::remove_dir_all(home.path().join("Applications/Cursor.app")).unwrap();
        std::fs::create_dir_all(home.path().join(".cursor")).unwrap();
        assert!(folders.has(cursor));
    }
}
